// Test-only independent Chrome file navigation. Never used by Yu production.
// Protocol contracts: https://chromedevtools.github.io/devtools-protocol/
// A fresh profile is mandatory; never attach to the user's browser/session.
import {spawn} from 'node:child_process';
import {readFile, writeFile, mkdir, open} from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';

const [manifestPath, outputPath] = process.argv.slice(2);
if (!manifestPath || !outputPath) throw new Error('Usage: node check-group5-browser.mjs manifest.json NEW_OUTPUT');
const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
if (!Array.isArray(manifest.cases) || manifest.cases.length < 1 || manifest.cases.length > 8) throw new Error('Expected 1..8 fixed browser cases');
const out = path.resolve(outputPath);
await mkdir(out); // Fail rather than reusing browser state or overwriting evidence.
const profile = path.join(out, 'isolated-profile');
const log = await open(path.join(out, 'chrome.log'), 'wx');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
if (!Number.isSafeInteger(manifest.producer_pid) || manifest.producer_pid <= 0) throw new Error('Missing exact producer PID');
try {
  process.kill(manifest.producer_pid, 0);
  throw new Error('The exporting test application has not exited');
} catch (error) {
  if (error.code !== 'ESRCH') throw error;
}
const chrome = spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--disable-extensions', '--disable-sync', 'about:blank'
], {stdio: ['ignore', 'ignore', log.fd]});
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let socket;
const results = [];
const report = {passed: false, evidence: 'independent_browser_file_navigation', producer_exited: true, cases: results};
try {
  let port;
  for (let n = 0; n < 100; n++) {
    try { port = (await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; }
    catch { if (chrome.exitCode !== null) throw new Error('Isolated Chrome exited'); await delay(100); }
  }
  if (!port) throw new Error('Isolated Chrome debugging endpoint unavailable');
  const endpoint = `http://127.0.0.1:${port}`;
  report.browser = await (await fetch(endpoint + '/json/version')).json();
  const pages = await (await fetch(endpoint + '/json/list')).json();
  const page = pages.find(page => page.type === 'page');
  if (!page) throw new Error('No isolated browser page');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {socket.onopen = resolve; socket.onerror = reject;});
  let sequence = 0;
  const pending = new Map();
  let requests = [], exceptions = [];
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const entry = pending.get(message.id);
      if (entry) {clearTimeout(entry.timer); pending.delete(message.id); message.error ? entry.reject(new Error(JSON.stringify(message.error))) : entry.resolve(message.result);}
    } else if (message.method === 'Network.requestWillBeSent') {
      const url = message.params.request.url;
      requests.push({scheme: url.split(':')[0], type: message.params.type});
    } else if (message.method === 'Runtime.exceptionThrown') {
      exceptions.push(message.params.exceptionDetails.text);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => {pending.delete(id); reject(new Error('CDP timeout: ' + method));}, 15000);
    pending.set(id, {resolve, reject, timer}); socket.send(JSON.stringify({id, method, params}));
  });
  const evaluate = async expression => {
    const answer = await send('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true});
    if (answer.exceptionDetails) throw new Error(JSON.stringify(answer.exceptionDetails));
    return answer.result.value;
  };
  await send('Page.enable'); await send('Runtime.enable'); await send('Network.enable');
  await send('Network.setCacheDisabled', {cacheDisabled: true});
  await send('Network.setBypassServiceWorker', {bypass: true});
  await send('Network.emulateNetworkConditions', {offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0});
  await send('Emulation.setDeviceMetricsOverride', {width: 1200, height: 900, deviceScaleFactor: 1, mobile: false});
  for (const test of manifest.cases) {
    if (!/^[a-z][a-z0-9-]*$/.test(test.id)) throw new Error('Invalid case ID');
    const file = path.resolve(test.path), bytes = await readFile(file);
    if (digest(bytes) !== test.sha256) throw new Error('Moved HTML bytes changed');
    requests = []; exceptions = [];
    const navigation = await send('Page.navigate', {url: pathToFileURL(file).href});
    if (navigation.errorText) throw new Error('Direct file navigation failed: ' + navigation.errorText);
    let loaded = false;
    for (let n = 0; n < 100; n++) {
      loaded = await evaluate(`location.href === ${JSON.stringify(pathToFileURL(file).href)} && document.readyState === "complete" && document.images.length === ${Number(test.images)} && [...document.images].every(i => i.complete && i.naturalWidth > 0 && i.naturalHeight > 0)`);
      if (loaded) break; await delay(100);
    }
    const state = await evaluate(`(() => {
      const ids = [...document.querySelectorAll('[id]')].map(n=>n.id);
      const links = [...document.querySelectorAll('a[href^="#"]')];
      return {url: location.href, online: navigator.onLine, title: document.title,
        images: [...document.images].map(i=>({width:i.naturalWidth,height:i.naturalHeight})),
        scripts: document.scripts.length, missingAnchors: links.filter(a=>!document.getElementById(a.hash.slice(1))).length,
        duplicateIds: ids.length - new Set(ids).size, detailsExpanded: [...document.querySelectorAll('details')].every(d=>d.open),
        text: document.body.innerText, height: document.documentElement.scrollHeight,
        foreground:getComputedStyle(document.documentElement).color, background:getComputedStyle(document.documentElement).backgroundColor};
    })()`);
    const errors = [];
    if (!loaded || state.images.length !== test.images) errors.push('resource load/count');
    if (state.url !== pathToFileURL(file).href) errors.push('not direct file URL');
    if (state.online || requests.some(r=>['http','https','ws','wss'].includes(r.scheme))) errors.push('offline/network contract');
    if (state.scripts || exceptions.length || state.duplicateIds || state.missingAnchors) errors.push('active content, exception or anchor identity');
    if (!state.detailsExpanded || !state.text.includes(test.marker)) errors.push('missing whole-document content');
    const anchors = await evaluate(`(() => {
      const links = ['.yu-toc a','.yu-notes a'].map(s=>document.querySelector(s)).filter(Boolean);
      return links.map(a=>{a.click(); return {expected:a.hash,actual:location.hash};});
    })()`);
    if (anchors.some(a=>a.expected !== a.actual)) errors.push('fragment navigation');
    const captures = [];
    const points = [{name:'top',y:0},{name:'middle',y:Math.floor(state.height/2)},{name:'bottom',y:state.height}];
    if (test.formulas) {
      const y = await evaluate('document.querySelector(".yu-figure")?.getBoundingClientRect().top + scrollY');
      if (Number.isFinite(y)) points.splice(1,0,{name:'formulas',y:Math.max(0,y-160)});
    }
    for (const point of points) {
      await evaluate(`scrollTo(0,${point.y})`); await delay(160);
      const image = await send('Page.captureScreenshot', {format:'png',captureBeyondViewport:false});
      const name = `${test.id}-${point.name}.png`, pixels=Buffer.from(image.data,'base64');
      await writeFile(path.join(out,name),pixels,{flag:'wx'}); captures.push({name,sha256:digest(pixels)});
    }
    delete state.text;
    results.push({id:test.id,passed:errors.length===0,errors,html_sha256:test.sha256,state,requests,exceptions,anchors,captures,
      visual_review:'not_automatically_passed'});
    await writeFile(path.join(out,'report.json'),JSON.stringify(report,null,2));
    if (errors.length) throw new Error(test.id + ': ' + errors.join(', '));
  }
  report.passed = true;
} finally {
  await writeFile(path.join(out,'report.json'),JSON.stringify(report,null,2));
  socket?.close();
  if (chrome.exitCode === null) chrome.kill('SIGTERM');
  await Promise.race([new Promise(resolve=>chrome.once('exit',resolve)),delay(5000)]);
  if (chrome.exitCode === null) chrome.kill('SIGKILL');
  await log.close();
}
console.log(JSON.stringify({passed:report.passed,cases:results.map(r=>({id:r.id,images:r.state.images.length,passed:r.passed}))}));
