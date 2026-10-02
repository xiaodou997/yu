param(
    [string]$Executable = (Join-Path $PSScriptRoot '../../../target/debug/yu-shell-windows.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../../artifacts/windows-group6/client')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName WindowsBase
$uiaReferences = @([Windows.Automation.AutomationElement].Assembly.Location, [Windows.Automation.AutomationEvent].Assembly.Location, [Windows.Rect].Assembly.Location) | Select-Object -Unique
Add-Type -ReferencedAssemblies $uiaReferences -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Automation;
public static class YuAccessibilityMessages {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string name, string title);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr parent, int id);
    [DllImport("user32.dll")] [return:MarshalAs(UnmanagedType.Bool)] public static extern bool MoveWindow(IntPtr hwnd, int x, int y, int width, int height, bool repaint);
    [DllImport("user32.dll")] public static extern int ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll", EntryPoint="GetClassLongPtrW")] public static extern IntPtr GetClassLongPtr(IntPtr hwnd, int index);
}
[ComImport, Guid("30cbe57d-d9d0-452a-ab13-7ac5ac4825ee"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface YuNativeAutomation {
    void CompareElements(IntPtr first, IntPtr second, out int same);
    void CompareRuntimeIds(IntPtr first, IntPtr second, out int same);
    void GetRootElement(out YuNativeElement element);
    void ElementFromHandle(IntPtr hwnd, out YuNativeElement element);
}
[ComImport, Guid("d22108aa-8ac5-49a5-837b-37bbb3d7591e"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface YuNativeElement {
    void SetFocus();
    void GetRuntimeId(out IntPtr array);
    void FindFirst(int scope, IntPtr condition, out IntPtr element);
    void FindAll(int scope, IntPtr condition, out IntPtr array);
    void FindFirstBuildCache(int scope, IntPtr condition, IntPtr cache, out IntPtr element);
    void FindAllBuildCache(int scope, IntPtr condition, IntPtr cache, out IntPtr array);
    void BuildUpdatedCache(IntPtr cache, out IntPtr element);
    void GetCurrentPropertyValue(int id, [MarshalAs(UnmanagedType.Struct)] out object value);
}
public static class YuNativeProperties {
    public static object Read(IntPtr hwnd, int id) {
        // CUIAutomation supplies the native standard-control proxies. The old
        // managed PowerShell host may otherwise expose Win32 controls as Pane.
        var client = (YuNativeAutomation)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("ff48dba4-60ef-4201-aa87-54103eef594e")));
        YuNativeElement element = null;
        try { client.ElementFromHandle(hwnd, out element); object value; element.GetCurrentPropertyValue(id, out value); return value; }
        finally { if (element != null) Marshal.FinalReleaseComObject(element); Marshal.FinalReleaseComObject(client); }
    }
}
public static class YuAccessibilityEvents {
    public static int Selection, Text, Structure, Focus, Layout;
    private static int ProcessId;
    public static void Register(AutomationElement document, int processId) {
        ProcessId = processId;
        Automation.AddAutomationEventHandler(TextPattern.TextSelectionChangedEvent, document, TreeScope.Element, (sender,args) => Interlocked.Increment(ref Selection));
        Automation.AddAutomationEventHandler(TextPattern.TextChangedEvent, document, TreeScope.Element, (sender,args) => Interlocked.Increment(ref Text));
        Automation.AddStructureChangedEventHandler(document, TreeScope.Subtree, (sender,args) => Interlocked.Increment(ref Structure));
        Automation.AddAutomationEventHandler(AutomationElement.LayoutInvalidatedEvent, document, TreeScope.Element, (sender,args) => Interlocked.Increment(ref Layout));
        Automation.AddAutomationFocusChangedEventHandler((sender,args) => {
            try { if (((AutomationElement)sender).Current.ProcessId == ProcessId) Interlocked.Increment(ref Focus); } catch (ElementNotAvailableException) {}
        });
    }
}
'@
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
$resolvedOutput = (Resolve-Path -LiteralPath $OutputDirectory).Path
$fixture = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'Fixtures/group6-accessibility.md'))
$inputPath = Join-Path $resolvedOutput ('uia-' + [guid]::NewGuid().ToString('N') + '.md')
[IO.File]::WriteAllText($inputPath, $fixture, [Text.UTF8Encoding]::new($false))
$null = New-Item -ItemType Directory -Path (Join-Path $resolvedOutput 'assets') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Fixtures/assets/yu-mark.png') -Destination (Join-Path $resolvedOutput 'assets/yu-mark.png') -Force
$sourceHash = (Get-FileHash -LiteralPath $inputPath).Hash
$process = Start-Process -FilePath $resolvedExecutable -ArgumentList ('"' + $inputPath + '"') -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $resolvedOutput 'stdout.log') -RedirectStandardError (Join-Path $resolvedOutput 'stderr.log')
$cases = [Collections.Generic.List[string]]::new()
function Check([bool]$condition, [string]$name) {
    if (-not $condition) { throw "FAIL: $name" }
    $cases.Add($name)
    Write-Host "PASS: $name"
}
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $surface = [IntPtr]::Zero
    while ($surface -eq [IntPtr]::Zero) {
        $process.Refresh()
        if ($process.HasExited) { throw "GUI exited: $($process.ExitCode)" }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Timed out waiting for editor HWND' }
        $surface = [YuAccessibilityMessages]::FindWindowEx($process.MainWindowHandle, [IntPtr]::Zero, 'YuEditorSurface', $null)
        Start-Sleep -Milliseconds 100
    }
    # Verify standard-control proxies before initializing the legacy managed
    # client, which replaces this process's native proxy factory table.
    $null = [YuAccessibilityMessages]::SendMessage($process.MainWindowHandle, 0x111, [IntPtr]::new(2003), [IntPtr]::Zero)
    $nativeDpi = [YuAccessibilityMessages]::GetDpiForWindow($process.MainWindowHandle)
    Check ([YuAccessibilityMessages]::GetClassLongPtr($process.MainWindowHandle, -14) -ne [IntPtr]::Zero -and [YuAccessibilityMessages]::GetClassLongPtr($process.MainWindowHandle, -34) -ne [IntPtr]::Zero) 'main window exposes embedded large and small Yu icons'
    $queryHwnd = [YuAccessibilityMessages]::GetDlgItem($process.MainWindowHandle, 2004)
    Check ([YuNativeProperties]::Read($queryHwnd, 30003) -eq 50004) 'native COM UIA resolves search as Edit control'
    Check (-not [string]::IsNullOrEmpty([YuNativeProperties]::Read($queryHwnd, 30005))) 'native search edit has an accessible name'
    Check ([YuAccessibilityMessages]::GetDlgItem($process.MainWindowHandle, 2003) -eq [IntPtr]::Zero) 'search is independent of the two sidebar tabs'
    foreach ($buttonId in @(2101, 2102, 2103, 2001, 2002, 2006, 2007, 2008)) {
        $buttonHwnd = [YuAccessibilityMessages]::GetDlgItem($process.MainWindowHandle, $buttonId)
        Check ([YuNativeProperties]::Read($buttonHwnd, 30003) -eq 50000) "native COM UIA resolves button $buttonId"
        Check (-not [string]::IsNullOrEmpty([YuNativeProperties]::Read($buttonHwnd, 30005))) "button $buttonId has an accessible name"
    }
    $document = [Windows.Automation.AutomationElement]::FromHandle($surface)
    Check ($document.Current.AutomationId -eq 'markdown-editor') 'external UIA resolves editor provider'
    Check ($document.Current.ControlType -eq [Windows.Automation.ControlType]::Document) 'editable document control type'
    [YuAccessibilityEvents]::Register($document, $process.Id)
    $text = $document.GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    $full = $text.DocumentRange
    Check ($full.GetText(-1) -ceq $fixture) 'TextPattern reads exact canonical Unicode Markdown'
    Check ($full.FindText('absent-uia-marker', $false, $false) -eq $null) 'no-match returns null successfully'
    Check ($full.FindText('WINDOWS ACCESSIBILITY', $false, $true).GetText(-1) -ceq 'Windows accessibility') 'case-insensitive search returns original source offsets'
    $emoji = $full.FindText([char]::ConvertFromUtf32(0x1f600), $false, $false)
    Check ($emoji.GetText(1) -eq '') 'bounded text never splits a surrogate pair'
    $emoji.Select()
    Check ($text.GetSelection()[0].GetText(-1) -ceq [char]::ConvertFromUtf32(0x1f600)) 'external range selection reaches editor state'
    Check ($emoji.GetBoundingRectangles().Count -gt 0) 'selection returns actual screen geometry'
    Check ($text.GetVisibleRanges().Count -gt 0) 'visible text ranges are exposed'
    $clone = $emoji.Clone()
    Check ($clone.Compare($emoji)) 'range clone and comparison across client COM wrappers'
    $heading = $document.FindFirst([Windows.Automation.TreeScope]::Children, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Windows accessibility'))
    Check ($null -ne $heading) 'source-backed heading in fragment tree'
    $headingRange = $text.RangeFromChild($heading)
    Check ($headingRange.GetText(-1).StartsWith('# Windows accessibility')) 'RangeFromChild crosses UIA marshaling correctly'
    Check ($full.FindAttribute([Windows.Automation.TextPatternIdentifiers]::IsReadOnlyAttribute, $false, $false).GetText(-1) -ceq $fixture) 'FindAttribute exposes editable canonical range'
    $task = $document.FindFirst([Windows.Automation.TreeScope]::Children, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Accessible task'))
    Check ($null -ne $task) 'task semantic node is named'
    $toggle = $task.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    Check ($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) 'unchecked task exposes toggle state'
    $toggle.Toggle()
    $current = $document.GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    Check ($current.DocumentRange.GetText(-1).Contains('- [x] Accessible task')) 'task action uses canonical transaction'
    $staleRejected = $false
    try { $null = $full.GetText(-1) } catch { $staleRejected = $true }
    Check $staleRejected 'old revision range is invalidated after edit'
    $null = [YuAccessibilityMessages]::SendMessage($process.MainWindowHandle, 0x111, [IntPtr]::new(1101), [IntPtr]::Zero)
    $text = $document.GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    Check ($text.DocumentRange.GetText(-1) -ceq $fixture) 'Undo restores task source exactly'
    $document.SetFocus()
    Check $document.Current.HasKeyboardFocus 'UIA SetFocus reaches editor HWND'
    $null = [YuAccessibilityMessages]::SendMessage($process.MainWindowHandle, 0x111, [IntPtr]::new(1202), [IntPtr]::Zero)
    Check (-not $document.Current.HasKeyboardFocus) 'F6 region action leaves editor and updates focus'
    $null = [YuAccessibilityMessages]::SendMessage($process.MainWindowHandle, 0x111, [IntPtr]::new(1203), [IntPtr]::Zero)
    Check $document.Current.HasKeyboardFocus 'Shift F6 region action returns to editor'
    $null = [YuAccessibilityMessages]::SendMessage($process.MainWindowHandle, 0x111, [IntPtr]::new(2003), [IntPtr]::Zero)
    $beforeBounds = $document.Current.BoundingRectangle
    Check ([YuAccessibilityMessages]::MoveWindow($process.MainWindowHandle, 80, 80, 1100, 700, $true)) 'independent fixture window moves and resizes'
    $afterBounds = $document.Current.BoundingRectangle
    Check ($beforeBounds.Width -ne $afterBounds.Width -or $beforeBounds.Height -ne $afterBounds.Height) 'external UIA geometry updates after native resize'
    Check ($text.DocumentRange.GetText(-1) -ceq $fixture) 'native resize preserves canonical Markdown'
    $null = [YuAccessibilityMessages]::ShowWindow($process.MainWindowHandle, 0)
    Check $document.Current.IsOffscreen 'hidden editor reports offscreen to external UIA'
    Check ($text.GetVisibleRanges().Count -eq 0) 'hidden editor exposes no visible text ranges'
    $null = [YuAccessibilityMessages]::ShowWindow($process.MainWindowHandle, 4)
    Check (-not $document.Current.IsOffscreen -and $text.GetVisibleRanges().Count -gt 0) 'showing editor restores visible UIA geometry'
    Check ((Get-FileHash -LiteralPath $inputPath).Hash -eq $sourceHash) 'UIA reads and navigation never write fixture to disk'
    $eventDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $eventDeadline -and ([YuAccessibilityEvents]::Selection -eq 0 -or [YuAccessibilityEvents]::Text -eq 0 -or [YuAccessibilityEvents]::Structure -eq 0 -or [YuAccessibilityEvents]::Focus -eq 0 -or [YuAccessibilityEvents]::Layout -eq 0)) { Start-Sleep -Milliseconds 50 }
    Check ([YuAccessibilityEvents]::Selection -gt 0) 'external client receives selection notifications'
    Check ([YuAccessibilityEvents]::Text -gt 0) 'external client receives text notifications'
    Check ([YuAccessibilityEvents]::Structure -gt 0) 'external client receives semantic tree notifications'
    Check ([YuAccessibilityEvents]::Focus -gt 0) 'external client receives keyboard focus notifications'
    Check ([YuAccessibilityEvents]::Layout -gt 0) 'external client receives native geometry notifications'
    [Windows.Automation.Automation]::RemoveAllEventHandlers()
    Stop-Process -Id $process.Id
    $process.WaitForExit(5000) | Out-Null
    $closedRejected = $false
    try { $null = $text.DocumentRange.GetText(-1) } catch { $closedRejected = $true }
    Check $closedRejected 'external provider becomes unavailable after fixture process exits'
    $report = [pscustomobject]@{ status='PASS'; cases=$cases.ToArray(); executable=$resolvedExecutable; sha256=(Get-FileHash -LiteralPath $resolvedExecutable).Hash; input=$inputPath; input_sha256=$sourceHash; process_id=$process.Id; native_dpi=$nativeDpi; events=@{selection=[YuAccessibilityEvents]::Selection; text=[YuAccessibilityEvents]::Text; structure=[YuAccessibilityEvents]::Structure; focus=[YuAccessibilityEvents]::Focus; layout=[YuAccessibilityEvents]::Layout}; actual_uia_client='Native CUIAutomation plus System.Windows.Automation in separate PowerShell process'; narrator_speech='NOT_RUN'; system_contrast_switch='NOT_RUN' }
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $resolvedOutput 'results.json') -Encoding UTF8
} catch {
    [pscustomobject]@{status='FAIL'; cases=$cases.ToArray(); error=$_.Exception.Message; input=$inputPath} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $resolvedOutput 'results.json') -Encoding UTF8
    throw
} finally {
    [Windows.Automation.Automation]::RemoveAllEventHandlers()
    $process.Refresh()
    if (-not $process.HasExited) {
        # Only the independent fixture process created above is stopped. User
        # document windows and pre-existing Narrator instances are untouched.
        Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
        $process.WaitForExit(5000) | Out-Null
    }
}
