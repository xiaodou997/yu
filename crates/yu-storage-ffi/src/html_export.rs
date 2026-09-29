//! macOS HTML export task. No mutable editor/session crosses this boundary.
//! Every task owns its snapshot, resource bytes, helper and destination lease.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use yu_assets::{EmbeddedRenderRequest, EmbeddedResourceKind, EmbeddedStyle};
use yu_embedded_client::{NativeRendererClient, RenderControl, RenderFailure};
use yu_export::document::{
    ExportImage, HtmlDocument, HtmlOptions, HtmlResources, ResourceError, export_html_document,
};
use yu_export::portable::{
    Destination, FrozenImages, MAX_IMAGE_PIXELS, ProtectedFile, data_image, validate_svg,
    validate_svg_resource,
};
use yu_markdown::{EmbeddedKind, EmbeddedSpan};
use yu_text::TextSnapshot;

const MAX_TASKS: usize = 2;
#[cfg(test)]
#[path = "html_export_bounds.rs"]
mod bounds;
const TASK_TIMEOUT: Duration = Duration::from_secs(300);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
struct Ready {
    document: HtmlDocument,
    pdf: Option<yu_render_macos::RenderedPdf>,
    png: Option<yu_render_macos::PngPlan>,
    segments_destination: Option<Destination>,
    destination: Destination,
    protected: Vec<ProtectedFile>,
}
struct State {
    phase: &'static str,
    message: String,
    warnings: Vec<String>,
    images: usize,
    embedded: usize,
    pages: u32,
    ready: Option<Ready>,
    print_target: Option<Destination>,
    print_protected: Vec<ProtectedFile>,
    print_plan: Option<yu_render_macos::PrintPagePlan>,
    png_sizes: Vec<[u32; 2]>,
    output_path: Option<PathBuf>,
}
pub struct HtmlJob {
    control: RenderControl,
    revision: u64,
    preparation_deadline: Instant,
    pdf_settings: Option<yu_export::paged::PageSettings>,
    state: Mutex<State>,
    counted: bool,
    printing: bool,
    png_settings: Option<yu_export::png::PngSettings>,
    target_path: PathBuf,
}
impl Drop for HtmlJob {
    fn drop(&mut self) {
        self.control.close();
        if self.counted {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl HtmlJob {
    pub fn start(
        snapshot: TextSnapshot,
        source_path: PathBuf,
        target: &Path,
        config: &Value,
    ) -> Result<Arc<Self>, String> {
        let pdf_settings = match config.get("exportFormat").and_then(Value::as_str) {
            None | Some("html" | "png") => None,
            Some("pdf") => Some(yu_export::paged::PageSettings::from_config(config)?),
            _ => return Err("未知导出格式".into()),
        };
        let png_settings = if config.get("exportFormat").and_then(Value::as_str) == Some("png") {
            Some(yu_export::png::PngSettings::from_config(config)?)
        } else {
            None
        };
        let revision = snapshot.revision().get();
        let control = RenderControl::default();
        control.set_revision(revision);
        let previous = ACTIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_TASKS).then_some(count + 1)
        });
        if previous.is_err() {
            return Err("已有两个导出任务，请先完成或取消其中一个".into());
        }
        let job = Arc::new(Self {
            revision,
            control,
            preparation_deadline: Instant::now() + TASK_TIMEOUT,
            pdf_settings,
            counted: true,
            png_settings,
            target_path: target.to_path_buf(),
            printing: pdf_settings.is_some()
                && config.get("printPreparation").and_then(Value::as_bool) == Some(true),
            state: Mutex::new(State {
                phase: "preparing",
                message: "准备文档快照".into(),
                warnings: Vec::new(),
                images: 0,
                embedded: 0,
                pages: 0,
                ready: None,
                print_target: None,
                print_protected: Vec::new(),
                print_plan: None,
                png_sizes: Vec::new(),
                output_path: None,
            }),
        });
        let setup = (|| {
            if snapshot.len_bytes().get() > yu_export::document::MAX_SOURCE_BYTES as u64 {
                return Err("文档超过 8 MiB 导出预算".into());
            }
            let unsigned = |key: &str, default: u32| -> Result<u32, String> {
                match config.get(key) {
                    None => Ok(default),
                    Some(value) => value
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| "导出颜色或尺寸参数无效".into()),
                }
            };
            let defaults = HtmlOptions::default();
            let mut options = HtmlOptions {
                title: config
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Yu 文档")
                    .to_owned(),
                foreground: unsigned("foreground", defaults.foreground)?,
                background: unsigned("background", defaults.background)?,
                link: unsigned("link", defaults.link)?,
                width: unsigned("width", defaults.width)?,
                font_size: config
                    .get("fontSize")
                    .and_then(Value::as_f64)
                    .unwrap_or(16.0) as f32,
                reference_day: config
                    .get("referenceDay")
                    .and_then(Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("缺少固定日期上下文")?,
                dark: config.get("dark").and_then(Value::as_bool).unwrap_or(false),
            };
            if pdf_settings.is_some() {
                options.foreground = 0x24292fff;
                options.background = 0xffffffff;
                options.link = 0x1f4794ff;
                options.font_size = 16.0;
                options.dark = false;
            }
            let basis = if config.get("untitled").and_then(Value::as_bool) == Some(true) {
                config
                    .get("resourceBase")
                    .and_then(Value::as_str)
                    .map(|directory| Path::new(directory).join("untitled.md"))
            } else {
                Some(source_path.clone())
            };
            let destination = Destination::capture(
                target,
                config
                    .get("replaceExisting")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?;
            destination.validate(&[ProtectedFile::capture(&source_path)])?;
            Ok((options, basis, destination))
        })();
        let (options, basis, destination) = match setup {
            Ok(value) => value,
            Err(error) => {
                job.fail(error);
                return Ok(job);
            }
        };
        let worker = job.clone();
        let launch = std::thread::Builder::new()
            .name("yu-html-export".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.prepare(snapshot, source_path, basis, destination, options)
                }));
                match result {
                    Ok(Ok(mut ready)) => {
                        let mut state = worker
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        if !worker.control.is_current(worker.revision) {
                            state.phase = "cancelled";
                            state.message = "已取消导出".into();
                            return;
                        }
                        state.phase = if ready.document.warnings.is_empty() {
                            "ready"
                        } else {
                            "warnings"
                        };
                        state.message = "准备完成，等待提交".into();
                        state.warnings = ready.document.warnings.clone();
                        state.images = ready.document.image_count;
                        state.embedded = ready.document.embedded_count;
                        state.pages = ready.pdf.as_ref().map_or(0, |pdf| pdf.pages);
                        if worker.printing {
                            state.print_protected = ready.protected.clone();
                            state.print_plan =
                                ready.pdf.as_mut().and_then(|pdf| pdf.print_plan.take());
                        }
                        if let Some(png) = &ready.png {
                            state.png_sizes = png.sizes.clone();
                            state.output_path = Some(
                                ready
                                    .segments_destination
                                    .as_ref()
                                    .unwrap_or(&ready.destination)
                                    .path()
                                    .to_path_buf(),
                            );
                            if png.sizes.len() > 1 {
                                state.phase = "split";
                                state.message = format!("PNG需分为{}段，等待确认", png.sizes.len());
                            }
                        }
                        state.ready = Some(ready);
                    }
                    Ok(Err(error)) => worker.fail(error),
                    Err(_) => worker.fail("导出工作线程异常；未提交目标文件".into()),
                }
            });
        if launch.is_err() {
            job.fail("无法启动导出工作线程".into());
        }
        Ok(job)
    }
    fn prepare(
        self: &Arc<Self>,
        snapshot: TextSnapshot,
        path: PathBuf,
        basis: Option<PathBuf>,
        destination: Destination,
        options: HtmlOptions,
    ) -> Result<Ready, String> {
        self.prepare_checkpoint_at(Instant::now())?;
        let document = yu_markdown::parse(&snapshot);
        self.prepare_checkpoint_at(Instant::now())?;
        let helper = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|directory| directory.join("../Helpers/yu-document-renderer"))
            })
            .unwrap_or_default();
        let mut resources = Resources {
            task: self.clone(),
            images: FrozenImages::new(basis, Some(&path)),
            normalized: HashMap::new(),
            renderer: NativeRendererClient::new(helper, NEXT_ID.fetch_add(1, Ordering::Relaxed)),
            ordinal: 0,
            revision: snapshot.revision(),
        };
        self.stage("固定本地图片资源");
        for image in yu_markdown::image_spans(&document, &snapshot, None) {
            resources.checkpoint()?;
            if let Some(destination) = yu_markdown::image_destination_text(
                &snapshot,
                image,
                document.reference_definitions(),
            ) {
                resources
                    .images
                    .capture(&destination)
                    .map_err(|error| match error {
                        ResourceError::Fatal(message) | ResourceError::Warning(message) => message,
                    })?;
            }
        }
        let mut html_options = options.clone();
        // Width here only belongs to the inert HTML transport stylesheet. PNG
        // lays out at its independently validated width in the native flow.
        if self.png_settings.is_some() {
            html_options.width = options.width.clamp(360, 1200);
        }
        let mut result = export_html_document(&document, &html_options, &mut resources)?;
        let pdf = if let Some(settings) = self.pdf_settings {
            self.stage("准备 PDF 页面内容");
            let packet = yu_export::paged::prepare_pdf_packet(&result, settings)?;
            resources.checkpoint()?;
            self.stage("分页并绘制 PDF");
            let checkpoint = || self.prepare_checkpoint_at(Instant::now()).is_ok();
            let rendered = if self.printing {
                yu_render_macos::export_print_pdf(&packet, checkpoint)
            } else {
                yu_render_macos::export_pdf(&packet, checkpoint)
            };
            resources.checkpoint()?;
            result.html.clear();
            Some(rendered?)
        } else {
            None
        };
        let png = if let Some(settings) = self.png_settings {
            self.stage("布局整文档 PNG 并检查像素预算");
            let packet = yu_export::png::prepare_png_packet(&result, settings, &options)?;
            let plan = yu_render_macos::prepare_png(&packet, || {
                self.prepare_checkpoint_at(Instant::now()).is_ok()
            })?;
            settings.validate_sizes(&plan.sizes)?;
            result.html.clear();
            Some(plan)
        } else {
            None
        };
        let segments_destination = if png.as_ref().is_some_and(|p| p.sizes.len() > 1) {
            let name = self
                .target_path
                .file_stem()
                .ok_or("PNG文件名无效")?
                .to_string_lossy();
            let path = self.target_path.with_file_name(format!("{name}-images"));
            let target = Destination::capture(&path, false)?;
            target.validate(&resources.images.protected)?;
            Some(target)
        } else {
            None
        };
        resources.images.verify()?;
        resources.checkpoint()?;
        destination.validate(&resources.images.protected)?;
        Ok(Ready {
            document: result,
            pdf,
            png,
            segments_destination,
            destination,
            protected: resources.images.protected.clone(),
        })
    }
    fn prepare_checkpoint_at(&self, now: Instant) -> Result<(), String> {
        if !self.control.is_current(self.revision) {
            return Err("已取消导出".into());
        }
        if now >= self.preparation_deadline {
            return Err("导出超过 300 秒任务预算".into());
        }
        Ok(())
    }
    fn stage(&self, message: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.message = message.to_owned();
    }
    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.ready = None;
        if self.control.is_current(self.revision) {
            state.phase = "failed";
            state.message = error;
        } else {
            state.phase = "cancelled";
            state.message = "已取消导出".into();
        }
    }
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(
            state.phase,
            "committing" | "completed" | "completed_with_warnings"
        ) {
            return;
        }
        self.control.close();
        state.ready = None;
        state.phase = "cancelled";
        state.print_target = None;
        state.message = "已取消导出".into();
    }
    /// # Safety
    /// Context is a live native printing context; calls are serialized by its owner.
    pub unsafe fn draw_print_page(&self, page: u32, context: *mut std::ffi::c_void) -> bool {
        if !self.control.is_current(self.revision) || context.is_null() {
            return false;
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .print_plan
            .as_ref()
            .is_some_and(|plan| unsafe { plan.draw(page, context) })
    }
    /// Capture a system-panel destination before its potentially long output.
    /// System PDF bytes are staged privately and then use the ordinary atomic
    /// publication guard, including the original snapshot's resource identities.
    pub fn print_output(self: &Arc<Self>, path: &Path, publish: bool) -> Result<(), String> {
        let result = self.print_output_inner(path, publish);
        if let Err(error) = &result {
            self.fail(error.clone());
        }
        result
    }
    fn print_output_inner(self: &Arc<Self>, path: &Path, publish: bool) -> Result<(), String> {
        if !self.printing {
            return Err("不是打印准备任务".into());
        }
        if !publish {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !matches!(state.phase, "completed" | "completed_with_warnings")
                || !self.control.is_current(self.revision)
            {
                return Err("打印内容尚未完成或已经取消".into());
            }
            // NSSavePanel in NSPrintPanel already obtained overwrite consent.
            let target = Destination::capture(path, true)?;
            target.validate(&state.print_protected)?;
            state.print_target = Some(target);
            state.phase = "print_ready";
            state.message = "等待系统打印输出".into();
            return Ok(());
        }
        let (destination, protected) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.phase != "print_ready" || !self.control.is_current(self.revision) {
                return Err("打印目的地未确认或已取消".into());
            }
            let target = state.print_target.take().ok_or("打印目的地丢失")?;
            state.phase = "writing";
            state.message = "检查并保存系统打印文件".into();
            (target, state.print_protected.clone())
        };
        let spool = path.to_path_buf();
        let worker = self.clone();
        std::thread::Builder::new()
            .name("yu-print-publish".into())
            .spawn(move || {
                let result = (|| -> Result<(), String> {
                    use std::io::Read;
                    let mut file =
                        std::fs::File::open(&spool).map_err(|_| "系统打印文件不可读取")?;
                    let length = file.metadata().map_err(|_| "不能检查系统打印文件")?.len();
                    if length == 0 || length > yu_export::document::MAX_OUTPUT_BYTES as u64 {
                        return Err("系统打印文件超过 256 MiB 预算或为空".into());
                    }
                    let mut bytes = vec![0; length as usize];
                    for part in bytes.chunks_mut(1024 * 1024) {
                        if !worker.control.is_current(worker.revision) {
                            return Err("已取消打印".into());
                        }
                        file.read_exact(part)
                            .map_err(|_| "系统打印文件读取失败或长度改变")?;
                    }
                    if file
                        .read(&mut [0u8; 1])
                        .map_err(|_| "系统打印文件读取失败")?
                        != 0
                        || !bytes.starts_with(b"%PDF-")
                    {
                        return Err("系统打印文件无效或长度改变".into());
                    }
                    destination.publish_guarded(
                        &bytes,
                        &protected,
                        || {
                            if worker.control.is_current(worker.revision) {
                                Ok(())
                            } else {
                                Err("已取消打印".into())
                            }
                        },
                        || {
                            let mut state = worker
                                .state
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            if !worker.control.is_current(worker.revision) {
                                return Err("已取消打印".into());
                            }
                            state.phase = "committing";
                            Ok(state)
                        },
                    )
                })();
                match result {
                    Ok(()) => {
                        let mut state = worker
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        state.phase = if state.warnings.is_empty() {
                            "completed"
                        } else {
                            "completed_with_warnings"
                        };
                        state.message = "系统打印文件已保存".into();
                    }
                    Err(error) => worker.fail(error),
                }
            })
            .map_err(|_| "无法启动打印文件保存线程".to_owned())?;
        Ok(())
    }
    fn publish_png(&self, mut ready: Ready) -> Result<(), String> {
        let mut plan = ready.png.take().ok_or("PNG布局丢失")?;
        let count = plan.sizes.len();
        let check = || {
            if self.control.is_current(self.revision) {
                Ok(())
            } else {
                Err("已取消PNG导出".to_owned())
            }
        };
        let guard = || {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            check()?;
            state.phase = "committing";
            Ok(state)
        };
        if let Some(directory) = ready.segments_destination {
            directory.publish_png_directory(
                count,
                &ready.protected,
                |index, limit| {
                    self.stage(&format!("绘制并编码PNG {}/{}", index + 1, count));
                    plan.encode(index, limit, || check().is_ok())
                },
                check,
                guard,
                yu_render_macos::move_directory_exclusive,
            )
        } else {
            self.stage("绘制并编码整文档PNG");
            let bytes =
                plan.encode(0, yu_export::document::MAX_OUTPUT_BYTES, || check().is_ok())?;
            ready
                .destination
                .publish_guarded(&bytes, &ready.protected, check, guard)
        }
    }
    pub fn status_json(&self) -> String {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        json!({"phase": state.phase, "message": state.message, "revision": self.revision,
            "warnings": state.warnings, "images": state.images, "embedded": state.embedded,
            "pages": state.pages, "pngSizes":state.png_sizes,"outputPath":state.output_path,
            "format": if self.png_settings.is_some() { "png" } else if self.pdf_settings.is_some() { "pdf" } else { "html" }})
        .to_string()
    }
    pub fn commit(self: &Arc<Self>, allow_warnings: bool) -> Result<(), String> {
        let ready = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !self.control.is_current(self.revision) {
                return Err("导出已经取消".into());
            }
            if !state.warnings.is_empty() && !allow_warnings {
                return Err("需要明确确认带占位或诊断导出".into());
            }
            if !matches!(state.phase, "ready" | "warnings" | "split") {
                return Err("导出尚未准备完成".into());
            }
            let ready = state.ready.take().ok_or("导出任务结果丢失")?;
            state.phase = "writing";
            state.message = "写入并检查输出文件".into();
            ready
        };
        let worker = self.clone();
        std::thread::Builder::new()
            .name("yu-html-publish".into())
            .spawn(move || {
                let result = if ready.png.is_some() {
                    worker.publish_png(ready)
                } else {
                    ready.destination.publish_guarded(
                        ready
                            .pdf
                            .as_ref()
                            .map_or(ready.document.html.as_bytes(), |pdf| pdf.bytes.as_slice()),
                        &ready.protected,
                        || {
                            if worker.control.is_current(worker.revision) {
                                Ok(())
                            } else {
                                Err("已取消导出".into())
                            }
                        },
                        || {
                            let mut state = worker
                                .state
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            if !worker.control.is_current(worker.revision) {
                                return Err("已取消导出".into());
                            }
                            state.phase = "committing";
                            state.message = "正在提交完整文件".into();
                            Ok(state)
                        },
                    )
                };
                match result {
                    Ok(()) => {
                        let mut state = worker
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        state.phase = if state.warnings.is_empty() {
                            "completed"
                        } else {
                            "completed_with_warnings"
                        };
                        state.message = if state.warnings.is_empty() {
                            if worker.png_settings.is_some() {
                                "PNG 导出完成".into()
                            } else if worker.pdf_settings.is_some() {
                                "PDF 导出完成".into()
                            } else {
                                "HTML 导出完成".into()
                            }
                        } else {
                            "已带占位或诊断导出，请检查警告".into()
                        };
                    }
                    Err(error) => worker.fail(error),
                }
            })
            .map_err(|_| {
                self.fail("无法启动输出线程".into());
                "无法启动输出线程".to_owned()
            })?;
        Ok(())
    }
}
struct Resources {
    task: Arc<HtmlJob>,
    images: FrozenImages,
    normalized: HashMap<String, ExportImage>,
    renderer: NativeRendererClient,
    ordinal: usize,
    revision: yu_core::Revision,
}
impl HtmlResources for Resources {
    fn checkpoint(&mut self) -> Result<(), String> {
        self.task.prepare_checkpoint_at(Instant::now())
    }
    fn image(&mut self, destination: &str) -> Result<ExportImage, ResourceError> {
        if let Some(image) = self.normalized.get(destination) {
            return Ok(image.clone());
        }
        self.images.capture(destination)?;
        let bytes = self.images.bytes(destination)?;
        let svg = std::str::from_utf8(bytes).ok().filter(|value| {
            value
                .trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with('<')
        });
        let image = if let Some(svg) = svg {
            validate_svg_resource(svg)?;
            data_image(bytes, "image/svg+xml", 1, 1)
        } else {
            let (png, width, height) = yu_render_macos::export_image_png(bytes, MAX_IMAGE_PIXELS)
                .map_err(|budget| {
                if budget {
                    ResourceError::Fatal("图片超出 32 Mi 像素/编码预算".into())
                } else {
                    ResourceError::Warning("图片解码失败，未使用旧图".into())
                }
            })?;
            data_image(&png, "image/png", width, height)
        };
        self.normalized
            .insert(destination.to_owned(), image.clone());
        Ok(image)
    }
    fn embedded(
        &mut self,
        span: EmbeddedSpan,
        source: &str,
        options: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError> {
        self.ordinal += 1;
        self.task
            .stage(&format!("准备公式和图表：第 {} 项", self.ordinal));
        let kind = match span.kind {
            EmbeddedKind::Math => EmbeddedResourceKind::Math,
            EmbeddedKind::Mermaid => EmbeddedResourceKind::Mermaid,
        };
        let style = EmbeddedStyle::new(options.font_size, options.foreground, options.dark)
            .ok_or_else(|| ResourceError::Fatal("资源样式无效".into()))?
            .with_display(span.display)
            .with_reference_day(Some(options.reference_day));
        let request =
            EmbeddedRenderRequest::new(self.revision, span.source, kind, source.to_owned())
                .map_err(|_| ResourceError::Fatal("渲染请求无效".into()))?
                .with_style(style);
        let payload = self
            .renderer
            .render(&request, &self.task.control)
            .map_err(|error| match error {
                RenderFailure::InvalidSource(message) => ResourceError::Warning(message),
                RenderFailure::Cancelled => ResourceError::Fatal("已取消导出".into()),
                RenderFailure::Worker(_) => {
                    ResourceError::Fatal("内置公式/图表工作进程失败或超时；未使用旧图".into())
                }
            })?;
        let svg = payload
            .markup()
            .ok_or_else(|| ResourceError::Fatal("内置渲染器未提供静态 SVG".into()))?;
        validate_svg(svg)
            .map_err(|message| ResourceError::Fatal(format!("内置 SVG 安全检查失败：{message}")))?;
        let mut image = data_image(
            svg.as_bytes(),
            "image/svg+xml",
            payload.dimensions().width(),
            payload.dimensions().height(),
        );
        image.baseline_milli = payload.baseline_milli();
        Ok(image)
    }
}
