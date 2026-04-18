$ErrorActionPreference = "Stop"

function Read-Payload {
    $raw = [Console]::In.ReadToEnd()
    if ([string]::IsNullOrWhiteSpace($raw)) {
        return @{}
    }
    return ($raw | ConvertFrom-Json)
}

function Emit-Result($value) {
    @{ ok = $true; result = $value } | ConvertTo-Json -Depth 16
}

function Emit-Error($message) {
    @{ ok = $false; error = "$message" } | ConvertTo-Json -Depth 16
    exit 1
}

function Get-Field($payload, $name, $default = $null) {
    if ($null -eq $payload) { return $default }
    $prop = $payload.PSObject.Properties[$name]
    if ($null -eq $prop) { return $default }
    return $prop.Value
}

function Ensure-Assemblies {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -AssemblyName Microsoft.VisualBasic
}

function Ensure-Win32 {
    if ("ThinClawDesktop.Win32" -as [type]) {
        return
    }
    Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ThinClawDesktop {
    public static class Win32 {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [StructLayout(LayoutKind.Sequential)]
        public struct RECT {
            public int Left;
            public int Top;
            public int Right;
            public int Bottom;
        }
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr extraData);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder text, int count);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
        public const uint MOUSEEVENTF_LEFTUP = 0x0004;
        public const uint MOUSEEVENTF_WHEEL = 0x0800;
        public static List<WindowInfo> TopWindows() {
            var list = new List<WindowInfo>();
            EnumWindows((hWnd, lParam) => {
                if (!IsWindowVisible(hWnd)) return true;
                var title = new StringBuilder(512);
                GetWindowText(hWnd, title, title.Capacity);
                if (string.IsNullOrWhiteSpace(title.ToString())) return true;
                var klass = new StringBuilder(256);
                GetClassName(hWnd, klass, klass.Capacity);
                uint pid;
                GetWindowThreadProcessId(hWnd, out pid);
                RECT rect;
                GetWindowRect(hWnd, out rect);
                list.Add(new WindowInfo {
                    Handle = hWnd,
                    HandleString = "0x" + hWnd.ToInt64().ToString("X"),
                    Title = title.ToString(),
                    ClassName = klass.ToString(),
                    ProcessId = (int)pid,
                    X = rect.Left,
                    Y = rect.Top,
                    Width = Math.Max(1, rect.Right - rect.Left),
                    Height = Math.Max(1, rect.Bottom - rect.Top)
                });
                return true;
            }, IntPtr.Zero);
            return list;
        }
    }
    public class WindowInfo {
        public IntPtr Handle { get; set; }
        public string HandleString { get; set; }
        public string Title { get; set; }
        public string ClassName { get; set; }
        public int ProcessId { get; set; }
        public int X { get; set; }
        public int Y { get; set; }
        public int Width { get; set; }
        public int Height { get; set; }
    }
}
"@
}

function Test-Tesseract {
    return [bool](Get-Command tesseract -ErrorAction SilentlyContinue)
}

function Read-OcrText($path) {
    if (-not (Test-Tesseract)) {
        return @()
    }
    $text = & tesseract $path stdout 2>$null
    if ([string]::IsNullOrWhiteSpace($text)) {
        return @()
    }
    return @(@{
        text = ($text -join "`n").Trim()
        confidence = $null
        bounds = @{ x = 0; y = 0; width = 1; height = 1 }
    })
}

function Get-GenericUiProvider {
    return "notepad"
}

function Get-WindowInventory {
    Ensure-Win32
    $windows = [ThinClawDesktop.Win32]::TopWindows()
    $foreground = [ThinClawDesktop.Win32]::GetForegroundWindow().ToInt64()
    $result = @()
    foreach ($window in $windows) {
        $process = Get-Process -Id $window.ProcessId -ErrorAction SilentlyContinue
        $name = if ($process) { $process.ProcessName } else { $window.ClassName }
        $bundle = if ($process) { "$($process.ProcessName).exe" } else { $window.ClassName }
        $result += [pscustomobject]@{
            name = $name
            bundle_id = $bundle
            pid = $window.ProcessId
            active = ($window.Handle.ToInt64() -eq $foreground)
            hidden = $false
            window_id = $window.HandleString
            title = $window.Title
            bounds = [pscustomobject]@{
                x = $window.X
                y = $window.Y
                width = $window.Width
                height = $window.Height
            }
            target_ref = "window:$($window.HandleString)"
        }
    }
    return $result
}

function Resolve-WindowTarget($payload) {
    $windowId = Get-Field $payload "window_id"
    if (-not $windowId) {
        $targetRef = Get-Field $payload "target_ref"
        if ($targetRef -and "$targetRef".StartsWith("window:")) {
            $windowId = "$targetRef".Substring(7)
        }
    }
    $bundle = Get-Field $payload "bundle_id"
    $title = Get-Field $payload "title"
    $windows = Get-WindowInventory
    if ($windowId) {
        return $windows | Where-Object { $_.window_id -eq "$windowId" } | Select-Object -First 1
    }
    if ($bundle) {
        $match = $windows | Where-Object {
            $_.bundle_id -like "*$bundle*" -or $_.name -like "*$bundle*" -or $_.title -like "*$bundle*"
        } | Select-Object -First 1
        if ($match) { return $match }
    }
    if ($title) {
        return $windows | Where-Object { $_.title -like "*$title*" } | Select-Object -First 1
    }
    return $null
}

function Focus-Window($window) {
    if ($null -eq $window) {
        throw "could not resolve target window"
    }
    Ensure-Win32
    $handle = [IntPtr]([Convert]::ToInt64(($window.window_id -replace '^0x', ''), 16))
    [ThinClawDesktop.Win32]::ShowWindowAsync($handle, 5) | Out-Null
    [ThinClawDesktop.Win32]::SetForegroundWindow($handle) | Out-Null
    return @{
        focused = $true
        window_id = $window.window_id
        bundle_id = $window.bundle_id
    }
}

function Get-TargetCenter($window) {
    $bounds = $window.bounds
    $x = [int]($bounds.x + [Math]::Max(1, $bounds.width) / 2)
    $y = [int]($bounds.y + [Math]::Max(1, $bounds.height) / 2)
    return @{ x = $x; y = $y }
}

function Invoke-MouseClick($window, [int]$repeat = 1) {
    Ensure-Win32
    $point = Get-TargetCenter $window
    [ThinClawDesktop.Win32]::SetCursorPos($point.x, $point.y) | Out-Null
    for ($i = 0; $i -lt $repeat; $i++) {
        [ThinClawDesktop.Win32]::mouse_event([ThinClawDesktop.Win32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
        [ThinClawDesktop.Win32]::mouse_event([ThinClawDesktop.Win32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 60
    }
}

function Convert-ToSendKeysChord($modifiers, $key) {
    $prefix = ""
    foreach ($modifier in @($modifiers)) {
        switch ("$modifier".ToLowerInvariant()) {
            "ctrl" { $prefix += "^" }
            "control" { $prefix += "^" }
            "alt" { $prefix += "%" }
            "option" { $prefix += "%" }
            "shift" { $prefix += "+" }
            default { }
        }
    }
    $mapped = switch ("$key".ToLowerInvariant()) {
        "enter" { "{ENTER}" }
        "return" { "{ENTER}" }
        "tab" { "{TAB}" }
        "escape" { "{ESC}" }
        "esc" { "{ESC}" }
        "backspace" { "{BACKSPACE}" }
        "delete" { "{DELETE}" }
        default { "$key" }
    }
    return "$prefix$mapped"
}

function Invoke-TextEntry($payload, $text, [bool]$replace = $false) {
    Ensure-Assemblies
    $window = Resolve-WindowTarget $payload
    if ($window) {
        Focus-Window $window | Out-Null
    }
    if ($replace) {
        [System.Windows.Forms.SendKeys]::SendWait("^a")
        Start-Sleep -Milliseconds 50
        [System.Windows.Forms.SendKeys]::SendWait("{BACKSPACE}")
        Start-Sleep -Milliseconds 50
    }
    [System.Windows.Forms.SendKeys]::SendWait([string]$text)
}

function Get-AutomationTree($element, [int]$depth = 0, [int]$maxDepth = 2) {
    if ($null -eq $element -or $depth -gt $maxDepth) {
        return $null
    }
    $node = @{
        role = $element.Current.ControlType.ProgrammaticName
        name = $element.Current.Name
        automation_id = $element.Current.AutomationId
        target_ref = "automation:$($element.GetHashCode())"
        children = @()
    }
    if ($depth -lt $maxDepth) {
        $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
        $child = $walker.GetFirstChild($element)
        while ($child -ne $null) {
            $childNode = Get-AutomationTree $child ($depth + 1) $maxDepth
            if ($childNode) {
                $node.children += $childNode
            }
            $child = $walker.GetNextSibling($child)
        }
    }
    return $node
}

function Test-OutlookAvailable {
    try {
        $app = New-Object -ComObject Outlook.Application
        if ($null -ne $app) { $null = $app.GetNamespace("MAPI") }
        return $true
    } catch {
        return $false
    }
}

function Test-ExcelAvailable {
    try {
        $app = New-Object -ComObject Excel.Application
        if ($null -ne $app) { $app.Quit() }
        return $true
    } catch {
        return $false
    }
}

function Test-WordAvailable {
    try {
        $app = New-Object -ComObject Word.Application
        if ($null -ne $app) { $app.Quit() }
        return $true
    } catch {
        return $false
    }
}

function Get-OutlookApp {
    return New-Object -ComObject Outlook.Application
}

function Get-OutlookCalendarFolder($name, [bool]$createIfMissing = $false) {
    $app = Get-OutlookApp
    $ns = $app.GetNamespace("MAPI")
    $defaultFolder = $ns.GetDefaultFolder(9)
    if ([string]::IsNullOrWhiteSpace($name)) {
        return $defaultFolder
    }
    $root = $defaultFolder.Parent
    foreach ($folder in $root.Folders) {
        if ($folder.Name -eq $name) {
            return $folder
        }
    }
    if ($createIfMissing) {
        return $root.Folders.Add($name, 9)
    }
    return $null
}

function Find-OutlookItems($folder, $query) {
    $matches = @()
    foreach ($item in $folder.Items) {
        if ($item.MessageClass -notlike "IPM.Appointment*") { continue }
        $subject = [string]$item.Subject
        $body = [string]$item.Body
        if ($subject.ToLower().Contains($query.ToLower()) -or $body.ToLower().Contains($query.ToLower())) {
            $matches += @{
                id = $item.EntryID
                title = $subject
                start = ([DateTime]$item.Start).ToString("o")
                end = ([DateTime]$item.End).ToString("o")
                calendar = $folder.Name
            }
        }
    }
    return $matches
}

function Get-ExcelApp {
    try {
        return [Runtime.InteropServices.Marshal]::GetActiveObject("Excel.Application")
    } catch {
        $app = New-Object -ComObject Excel.Application
        $app.Visible = $true
        return $app
    }
}

function Get-ExcelWorkbook($payload) {
    $app = Get-ExcelApp
    $path = Get-Field $payload "path"
    if ($path) {
        $resolved = (Resolve-Path $path -ErrorAction SilentlyContinue)
        foreach ($wb in $app.Workbooks) {
            if ($resolved -and $wb.FullName -eq $resolved.Path) {
                $wb.Activate()
                return $wb
            }
        }
        return $app.Workbooks.Open($path)
    }
    if ($app.Workbooks.Count -lt 1) {
        throw "no active Excel workbook"
    }
    return $app.ActiveWorkbook
}

function Get-ExcelWorksheet($workbook) {
    $sheet = $workbook.ActiveSheet
    if ($null -eq $sheet) {
        $sheet = $workbook.Worksheets.Item(1)
        $sheet.Activate()
    }
    return $sheet
}

function Ensure-ExcelTable($worksheet, $tableName) {
    try {
        return $worksheet.ListObjects.Item($tableName)
    } catch {
        $range = $worksheet.UsedRange
        if ($range.Rows.Count -lt 2) {
            $worksheet.Range("A1").Value2 = "Column1"
            $worksheet.Range("B1").Value2 = "Column2"
            $worksheet.Range("A2").Value2 = ""
            $worksheet.Range("B2").Value2 = ""
            $range = $worksheet.Range("A1:B2")
        }
        $table = $worksheet.ListObjects.Add(1, $range, $null, 1)
        $table.Name = $tableName
        return $table
    }
}

function Get-WordApp {
    try {
        return [Runtime.InteropServices.Marshal]::GetActiveObject("Word.Application")
    } catch {
        $app = New-Object -ComObject Word.Application
        $app.Visible = $true
        return $app
    }
}

function Get-WordDocument($payload) {
    $app = Get-WordApp
    $path = Get-Field $payload "path"
    if ($path) {
        $resolved = (Resolve-Path $path -ErrorAction SilentlyContinue)
        foreach ($doc in $app.Documents) {
            if ($resolved -and $doc.FullName -eq $resolved.Path) {
                $doc.Activate()
                return $doc
            }
        }
        return $app.Documents.Open($path)
    }
    if ($app.Documents.Count -lt 1) {
        throw "no active Word document"
    }
    return $app.ActiveDocument
}

function Capture-Screen($payload) {
    Ensure-Assemblies
    $path = Get-Field $payload "path"
    if (-not $path) {
        $path = Join-Path ([IO.Path]::GetTempPath()) ("thinclaw-screen-{0}.png" -f ([Guid]::NewGuid().ToString("N")))
    }
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($screen.Location, [System.Drawing.Point]::Empty, $screen.Size)
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    return @{ path = $path }
}

function Invoke-Apps($payload) {
    $action = Get-Field $payload "action" "list"
    switch ($action) {
        "list" {
            return @(Get-WindowInventory)
        }
        "open" {
            $path = Get-Field $payload "path"
            if (-not $path) { throw "desktop_apps open requires path" }
            if ([IO.Path]::GetExtension($path).ToLowerInvariant() -eq ".txt") {
                Start-Process -FilePath "$env:WINDIR\System32\notepad.exe" -ArgumentList @($path) | Out-Null
                return @{ opened = $true; path = $path; provider = "notepad" }
            }
            Start-Process -FilePath $path | Out-Null
            return @{ opened = $true; path = $path }
        }
        "focus" {
            $window = Resolve-WindowTarget $payload
            if ($window) {
                return (Focus-Window $window)
            }
            $bundle = Get-Field $payload "bundle_id"
            if (-not $bundle) { throw "desktop_apps focus requires bundle_id or window_id" }
            [Microsoft.VisualBasic.Interaction]::AppActivate(($bundle -replace '\.exe$', '')) | Out-Null
            return @{ focused = $true; bundle_id = $bundle }
        }
        "quit" {
            $bundle = Get-Field $payload "bundle_id"
            if (-not $bundle) { throw "desktop_apps quit requires bundle_id" }
            Get-Process -Name ($bundle -replace '\.exe$', '') -ErrorAction SilentlyContinue | Stop-Process -Force
            return @{ quit = $true; bundle_id = $bundle }
        }
        "windows" {
            $bundle = Get-Field $payload "bundle_id"
            $windows = @(Get-WindowInventory)
            if ($bundle) {
                $windows = @($windows | Where-Object { $_.bundle_id -like "*$bundle*" -or $_.name -like "*$bundle*" })
            }
            return $windows
        }
        "menus" { return @() }
        default { throw "unsupported desktop_apps action $action" }
    }
}

function Invoke-UI($payload) {
    Ensure-Assemblies
    $action = Get-Field $payload "action" "snapshot"
    switch ($action) {
        "snapshot" {
            $focused = Resolve-WindowTarget @{ window_id = ("0x" + [ThinClawDesktop.Win32]::GetForegroundWindow().ToInt64().ToString("X")) }
            $rootElement = [System.Windows.Automation.AutomationElement]::RootElement
            return @{
                session_id = (Get-Field $payload "session_id" "desktop-main-session")
                tree = (Get-AutomationTree $rootElement 0 2)
                window_id = $(if ($focused) { $focused.window_id } else { $null })
                timestamp = [DateTime]::UtcNow.ToString("o")
            }
        }
        "click" {
            $window = Resolve-WindowTarget $payload
            if ($null -eq $window) { throw "desktop_ui click could not resolve a target" }
            Focus-Window $window | Out-Null
            Invoke-MouseClick $window 1
            return @{ success = $true; target_ref = $window.target_ref }
        }
        "double_click" {
            $window = Resolve-WindowTarget $payload
            if ($null -eq $window) { throw "desktop_ui double_click could not resolve a target" }
            Focus-Window $window | Out-Null
            Invoke-MouseClick $window 2
            return @{ success = $true; target_ref = $window.target_ref }
        }
        "type_text" {
            Invoke-TextEntry $payload ([string](Get-Field $payload "text" "")) $false
            return @{ success = $true }
        }
        "set_value" {
            $text = [string](Get-Field $payload "value" (Get-Field $payload "text" ""))
            Invoke-TextEntry $payload $text $true
            return @{ success = $true }
        }
        "keypress" {
            $key = Convert-ToSendKeysChord @() ([string](Get-Field $payload "key" ""))
            [System.Windows.Forms.SendKeys]::SendWait($key)
            return @{ success = $true }
        }
        "chord" {
            $key = Convert-ToSendKeysChord (Get-Field $payload "modifiers" @()) ([string](Get-Field $payload "key" ""))
            [System.Windows.Forms.SendKeys]::SendWait($key)
            return @{ success = $true }
        }
        "select_menu" {
            $window = Resolve-WindowTarget $payload
            if ($window) { Focus-Window $window | Out-Null }
            $path = Get-Field $payload "menu_path" (Get-Field $payload "value" @())
            if ($path -is [string]) {
                $path = @($path -split '>' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
            }
            foreach ($label in @($path)) {
                [System.Windows.Forms.SendKeys]::SendWait("%")
                Start-Sleep -Milliseconds 75
                [System.Windows.Forms.SendKeys]::SendWait($label.Substring(0, 1))
                Start-Sleep -Milliseconds 75
            }
            return @{ success = $true; menu_path = $path }
        }
        "scroll" {
            Ensure-Win32
            $amount = [int](Get-Field $payload "amount" 1)
            [ThinClawDesktop.Win32]::mouse_event([ThinClawDesktop.Win32]::MOUSEEVENTF_WHEEL, 0, 0, [uint32]($amount * 120), [UIntPtr]::Zero)
            return @{ success = $true; amount = $amount }
        }
        "drag" {
            Ensure-Win32
            $window = Resolve-WindowTarget $payload
            if ($null -eq $window) { throw "desktop_ui drag could not resolve a target" }
            $start = Get-TargetCenter $window
            $destination = Get-Field $payload "destination" @{}
            $endX = [int](Get-Field $destination "x" $start.x)
            $endY = [int](Get-Field $destination "y" $start.y)
            [ThinClawDesktop.Win32]::SetCursorPos($start.x, $start.y) | Out-Null
            [ThinClawDesktop.Win32]::mouse_event([ThinClawDesktop.Win32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 60
            [ThinClawDesktop.Win32]::SetCursorPos($endX, $endY) | Out-Null
            Start-Sleep -Milliseconds 60
            [ThinClawDesktop.Win32]::mouse_event([ThinClawDesktop.Win32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
            return @{ success = $true }
        }
        "wait_for" {
            $timeoutMs = [int](Get-Field $payload "timeout_ms" 250)
            $deadline = [DateTime]::UtcNow.AddMilliseconds([Math]::Max($timeoutMs, 50))
            while ([DateTime]::UtcNow -lt $deadline) {
                $window = Resolve-WindowTarget $payload
                if ($window) {
                    return @{ success = $true; target_ref = $window.target_ref }
                }
                Start-Sleep -Milliseconds 100
            }
            return @{
                success = $false
                retryable = $true
                error_code = "target_not_found"
                error_message = "desktop_ui wait_for timed out before the requested target appeared"
            }
        }
        default {
            return @{
                success = $false
                retryable = $true
                error_code = "not_implemented"
                error_message = "ui action $action is not implemented yet in the Windows sidecar"
            }
        }
    }
}

function Invoke-Screen($payload) {
    $action = Get-Field $payload "action" "capture"
    switch ($action) {
        "capture" { return (Capture-Screen $payload) }
        "window_capture" { return (Capture-Screen $payload) }
        "ocr" {
            $path = Get-Field $payload "path"
            if (-not $path) { $path = (Capture-Screen $payload).path }
            return @{ path = $path; ocr_blocks = @(Read-OcrText $path) }
        }
        "find_text" {
            $query = [string](Get-Field $payload "query" "")
            $path = Get-Field $payload "path"
            if (-not $path) { $path = (Capture-Screen $payload).path }
            $matches = @(Read-OcrText $path | Where-Object { $_.text.ToLower().Contains($query.ToLower()) })
            return @{ path = $path; matches = $matches }
        }
        default { throw "unsupported desktop_screen action $action" }
    }
}

function Invoke-Calendar($payload) {
    $action = Get-Field $payload "action" "list"
    $calendarName = [string](Get-Field $payload "calendar" (Get-Field $payload "calendar_title" "Calendar"))
    switch ($action) {
        "ensure_calendar" {
            $title = [string](Get-Field $payload "title" $calendarName)
            $folder = Get-OutlookCalendarFolder $title $true
            return @{ id = $folder.EntryID; title = $folder.Name; created = $true }
        }
        "list" {
            $folder = Get-OutlookCalendarFolder $calendarName $false
            if ($null -eq $folder) { return @() }
            $items = @()
            foreach ($item in $folder.Items) {
                if ($item.MessageClass -notlike "IPM.Appointment*") { continue }
                $items += @{
                    id = $item.EntryID
                    title = $item.Subject
                    start = ([DateTime]$item.Start).ToString("o")
                    end = ([DateTime]$item.End).ToString("o")
                    calendar = $folder.Name
                    notes = [string]$item.Body
                }
            }
            return $items
        }
        "find" {
            $folder = Get-OutlookCalendarFolder $calendarName $false
            if ($null -eq $folder) { return @() }
            return @(Find-OutlookItems $folder ([string](Get-Field $payload "query" "")))
        }
        "create" {
            $folder = Get-OutlookCalendarFolder $calendarName $true
            $item = $folder.Items.Add(1)
            $item.Subject = [string](Get-Field $payload "title" "Untitled Event")
            $item.Start = [DateTime](Get-Field $payload "start" ([DateTime]::UtcNow.ToString("o")))
            $item.End = [DateTime](Get-Field $payload "end" ([DateTime]::UtcNow.AddHours(1).ToString("o")))
            $item.Body = [string](Get-Field $payload "notes" "")
            $item.Save()
            return @{ id = $item.EntryID; title = $item.Subject; calendar = $folder.Name }
        }
        "update" {
            $app = Get-OutlookApp
            $item = $app.Session.GetItemFromID([string](Get-Field $payload "event_id" ""))
            if ($null -eq $item) { throw "event not found" }
            if (Get-Field $payload "title") { $item.Subject = [string](Get-Field $payload "title") }
            if (Get-Field $payload "notes") { $item.Body = [string](Get-Field $payload "notes") }
            if (Get-Field $payload "start") { $item.Start = [DateTime](Get-Field $payload "start") }
            if (Get-Field $payload "end") { $item.End = [DateTime](Get-Field $payload "end") }
            $item.Save()
            return @{ updated = $true; id = $item.EntryID }
        }
        "delete" {
            $app = Get-OutlookApp
            $item = $app.Session.GetItemFromID([string](Get-Field $payload "event_id" ""))
            if ($null -eq $item) { throw "event not found" }
            $id = $item.EntryID
            $item.Delete()
            return @{ deleted = $true; id = $id }
        }
        default { throw "unsupported calendar action $action" }
    }
}

function Invoke-Numbers($payload) {
    $action = Get-Field $payload "action" "open_doc"
    $app = Get-ExcelApp
    switch ($action) {
        "create_doc" {
            $path = [string](Get-Field $payload "path")
            if (-not $path) { throw "missing path" }
            $wb = $app.Workbooks.Add()
            $ws = $wb.Worksheets.Item(1)
            $ws.Name = "Sheet1"
            $ws.Range("A1").Value2 = "Column1"
            $ws.Range("B1").Value2 = "Column2"
            $ws.Range("A2").Value2 = ""
            $ws.Range("B2").Value2 = ""
            $table = $ws.ListObjects.Add(1, $ws.Range("A1:B2"), $null, 1)
            $table.Name = "Table_1"
            $table.DisplayName = "Table_1"
            $wb.SaveAs($path, 51)
            $wb.Activate()
            return @{ created = $true; path = $path; table = "Table 1" }
        }
        "open_doc" {
            $wb = Get-ExcelWorkbook $payload
            $wb.Activate()
            return @{ opened = $true; path = $wb.FullName }
        }
        "read_range" {
            $wb = Get-ExcelWorkbook $payload
            $ws = Get-ExcelWorksheet $wb
            $cell = [string](Get-Field $payload "cell" "A1")
            return @{ cell = $cell; value = [string]$ws.Range($cell).Text }
        }
        "write_range" {
            $wb = Get-ExcelWorkbook $payload
            $ws = Get-ExcelWorksheet $wb
            $cell = [string](Get-Field $payload "cell" "A1")
            $ws.Range($cell).Value2 = [string](Get-Field $payload "value" "")
            $wb.Save()
            return @{ written = $true; cell = $cell }
        }
        "set_formula" {
            $wb = Get-ExcelWorkbook $payload
            $ws = Get-ExcelWorksheet $wb
            $cell = [string](Get-Field $payload "cell" "A1")
            $ws.Range($cell).Formula = [string](Get-Field $payload "value" "")
            $wb.Save()
            return @{ formula_set = $true; cell = $cell }
        }
        "run_table_action" {
            $wb = Get-ExcelWorkbook $payload
            $ws = Get-ExcelWorksheet $wb
            $table = Ensure-ExcelTable $ws "Table_1"
            $tableAction = [string](Get-Field $payload "table_action" "")
            $rowIndex = [int](Get-Field $payload "row_index" 1)
            $columnIndex = [int](Get-Field $payload "column_index" 1)
            switch ($tableAction) {
                "add_row_above" {
                    $table.ListRows.Add([Math]::Max(1, $rowIndex)) | Out-Null
                }
                "add_row_below" {
                    $table.ListRows.Add([Math]::Max(1, $rowIndex + 1)) | Out-Null
                }
                "delete_row" {
                    $table.ListRows.Item($rowIndex).Delete()
                }
                "add_column_before" {
                    $column = $table.ListColumns.Add([Math]::Max(1, $columnIndex))
                    $column.Name = "Column$columnIndex"
                }
                "add_column_after" {
                    $column = $table.ListColumns.Add([Math]::Max(1, $columnIndex + 1))
                    $column.Name = "Column$($columnIndex + 1)"
                }
                "delete_column" {
                    $table.ListColumns.Item($columnIndex).Delete()
                }
                "clear_range" {
                    $ws.Range([string](Get-Field $payload "range" "A1")).ClearContents() | Out-Null
                }
                "sort_column_ascending" {
                    $sortRange = $table.Range
                    $sort = $ws.Sort
                    $sort.SortFields.Clear()
                    $sort.SortFields.Add($table.ListColumns.Item($columnIndex).Range, 0, 1) | Out-Null
                    $sort.SetRange($sortRange)
                    $sort.Header = 1
                    $sort.Apply()
                }
                "sort_column_descending" {
                    $sortRange = $table.Range
                    $sort = $ws.Sort
                    $sort.SortFields.Clear()
                    $sort.SortFields.Add($table.ListColumns.Item($columnIndex).Range, 0, 2) | Out-Null
                    $sort.SetRange($sortRange)
                    $sort.Header = 1
                    $sort.Apply()
                }
                default {
                    return @{
                        success = $false
                        error_code = "unsupported_table_action"
                        table_action = $tableAction
                    }
                }
            }
            $wb.Save()
            return @{ success = $true; table_action = $tableAction; table = "Table 1" }
        }
        "export" {
            $wb = Get-ExcelWorkbook $payload
            $path = [string](Get-Field $payload "export_path")
            if (-not $path) { throw "missing export_path" }
            $wb.SaveAs($path, 6)
            return @{ exported = $true; path = $path }
        }
        default { throw "unsupported numbers action $action" }
    }
}

function Invoke-Pages($payload) {
    $action = Get-Field $payload "action" "open_doc"
    $app = Get-WordApp
    switch ($action) {
        "create_doc" {
            $path = [string](Get-Field $payload "path")
            if (-not $path) { throw "missing path" }
            $doc = $app.Documents.Add()
            $doc.SaveAs([ref]$path, [ref]16)
            $doc.Activate()
            return @{ created = $true; path = $path }
        }
        "open_doc" {
            $doc = Get-WordDocument $payload
            $doc.Activate()
            return @{ opened = $true; path = $doc.FullName }
        }
        "insert_text" {
            $doc = Get-WordDocument $payload
            $doc.Content.InsertAfter([string](Get-Field $payload "text" ""))
            $doc.Save()
            return @{ inserted = $true }
        }
        "replace_text" {
            $doc = Get-WordDocument $payload
            $find = $doc.Content.Find
            $find.Text = [string](Get-Field $payload "search" "")
            $find.Replacement.Text = [string](Get-Field $payload "replacement" "")
            $null = $find.Execute($find.Text, $false, $false, $false, $false, $false, $true, 1, $false, $find.Replacement.Text, 2)
            $doc.Save()
            return @{ replaced = $true }
        }
        "find" {
            $doc = Get-WordDocument $payload
            $content = [string]$doc.Content.Text
            $search = [string](Get-Field $payload "search" "")
            return @{ found = $content.Contains($search); query = $search }
        }
        "export" {
            $doc = Get-WordDocument $payload
            $path = [string](Get-Field $payload "export_path")
            if (-not $path) { throw "missing export_path" }
            $doc.ExportAsFixedFormat($path, 17)
            return @{ exported = $true; path = $path }
        }
        default { throw "unsupported pages action $action" }
    }
}

Ensure-Assemblies
Ensure-Win32

$command = $args[0]
$payload = Read-Payload

try {
    switch ($command) {
        "health" {
            Emit-Result @{
                ok = $true
                sidecar = "ThinClawDesktopBridge"
                platform = "windows"
                bridge_backend = "windows_powershell"
                providers = @{
                    calendar = "outlook"
                    numbers = "excel"
                    pages = "word"
                    generic_ui = (Get-GenericUiProvider)
                }
                session_name = $env:SESSIONNAME
                username = $env:USERNAME
                timestamp = [DateTime]::UtcNow.ToString("o")
            }
        }
        "permissions" {
            Emit-Result @{
                platform = "windows"
                accessibility = [Environment]::UserInteractive
                screen_recording = $true
                calendar = $(if (Test-OutlookAvailable) { "available" } else { "missing" })
                excel = $(if (Test-ExcelAvailable) { "available" } else { "missing" })
                word = $(if (Test-WordAvailable) { "available" } else { "missing" })
                ocr = $(if (Test-Tesseract) { "available" } else { "missing" })
                generic_ui = (Get-GenericUiProvider)
                session_name = $env:SESSIONNAME
            }
        }
        "apps" { Emit-Result (Invoke-Apps $payload) }
        "ui" { Emit-Result (Invoke-UI $payload) }
        "screen" { Emit-Result (Invoke-Screen $payload) }
        "calendar" { Emit-Result (Invoke-Calendar $payload) }
        "numbers" { Emit-Result (Invoke-Numbers $payload) }
        "pages" { Emit-Result (Invoke-Pages $payload) }
        default { Emit-Error "unsupported command $command" }
    }
} catch {
    Emit-Error $_.Exception.Message
}
