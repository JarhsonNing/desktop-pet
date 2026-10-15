param([Parameter(Mandatory = $true)][int]$PetId)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class MochiStorageDialog {
    public delegate bool Visitor(IntPtr h, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(Visitor visitor, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder name, int size);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint command);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
    public static bool CloseOwnedPicker(uint panelId, IntPtr owner) {
        bool found = false;
        EnumWindows((h, data) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            var name = new StringBuilder(128); GetClassName(h, name, name.Capacity);
            if (pid == panelId && name.ToString() == "#32770" && GetWindow(h, 4) == owner) {
                found = PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
# No desktop input injection: close only a modal owned by this fixture's panel.
$panel = @(Get-CimInstance Win32_Process | Where-Object {
    $_.ParentProcessId -eq $PetId -and $_.Name -eq 'desktop-pet-panel.exe'
})
if ($panel.Count -ne 1) { throw 'Expected one panel owned by the isolated pet' }
$owner = (Get-Process -Id $panel[0].ProcessId).MainWindowHandle
$closed = [MochiStorageDialog]::CloseOwnedPicker($panel[0].ProcessId, $owner)
if (-not $closed) { throw 'No native picker owned by the panel was found' }
'Owned native folder dialog cancelled'
