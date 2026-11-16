param([long]$Hwnd, [int]$OwnerPid)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Hwnd)
if (!$root -or $root.Current.ProcessId -ne $OwnerPid) { throw 'Settings window is not owned' }
$condition = [Windows.Automation.PropertyCondition]::new(
    [Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
foreach ($edit in $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)) {
    $pattern = $null
    if ($edit.Current.ProcessId -eq $OwnerPid -and
        $edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$pattern) -and
        $pattern.Current.Value -match '^(chrome://)?settings(?:/|$)') {
        [Console]::WriteLine($pattern.Current.Value)
        break
    }
}
