param(
    [Parameter(Mandatory = $true)][int]$PetId,
    [ValidateSet('state', 'minimize')][string]$Mode = 'state',
    [long]$WindowHandle = 0
)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MochiTestWindow {
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint processId);
}
'@
# Restrict every operation to the isolated test pet and its own panel child.
$panel = @(Get-CimInstance Win32_Process | Where-Object {
    $_.ParentProcessId -eq $PetId -and $_.Name -eq 'desktop-pet-panel.exe'
})
if ($panel.Count -ne 1) { throw 'Expected one panel owned by the test pet' }
$window = if ($WindowHandle) { [IntPtr]$WindowHandle } else { (Get-Process -Id $panel[0].ProcessId).MainWindowHandle }
if ($window -eq [IntPtr]::Zero) { throw 'Panel window is unavailable' }
[uint32]$ownerId = 0
[void][MochiTestWindow]::GetWindowThreadProcessId($window, [ref]$ownerId)
if ($ownerId -ne $panel[0].ProcessId) { throw 'Window no longer belongs to the isolated panel' }
if ($Mode -eq 'minimize') {
    [void][MochiTestWindow]::ShowWindowAsync($window, 6)
    $petWindow = (Get-Process -Id $PetId).MainWindowHandle
    [void][MochiTestWindow]::SetForegroundWindow($petWindow)
}
@{
    panelPid = $panel[0].ProcessId
    windowHandle = $window.ToInt64()
    minimized = [MochiTestWindow]::IsIconic($window)
    visible = [MochiTestWindow]::IsWindowVisible($window)
    foreground = [MochiTestWindow]::GetForegroundWindow() -eq $window
} | ConvertTo-Json -Compress
