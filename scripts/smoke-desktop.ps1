param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\maestro.exe'),
    [switch]$NativeOnly,
    [switch]$AllowModelRequest
)

$ErrorActionPreference = 'Stop'
if (-not $NativeOnly -and -not $AllowModelRequest) {
    throw 'Full UI testing may call your configured model and incur charges. Use -AllowModelRequest explicitly, or -NativeOnly.'
}
if (Get-Process -Name maestro -ErrorAction SilentlyContinue) {
    throw 'Close existing Maestro processes before running this smoke test.'
}
if (-not (Test-Path -LiteralPath $Executable)) {
    throw 'Build the release desktop application first: npm run tauri build'
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

function Wait-Until([scriptblock]$Predicate, [string]$Description, [int]$Seconds = 20) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $result = & $Predicate
        if ($result) { return $result }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "Timed out waiting for $Description"
}

function Find-Name($Root, [string]$Name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name
    )
    $Root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}

# Keep this script ASCII so Windows PowerShell 5.1 can read it without a UTF-8 BOM.
$readyText = 'Rust ' + (-join [char[]]@(0x5185, 0x6838, 0x5DF2, 0x5C31, 0x7EEA))
$startText = -join [char[]]@(0x53D1, 0x9001)
$promptText = -join [char[]]@(0x53D1, 0x9001, 0x6D88, 0x606F)
$newSessionText = -join [char[]]@(0x65B0, 0x5EFA, 0x4F1A, 0x8BDD)
$completedText = -join [char[]]@(0x5DF2, 0x5B8C, 0x6210)
$replyText = 'Agent ' + (-join [char[]]@(0x56DE, 0x590D))
$application = $null
$previousBrowserArguments = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS

try {
    # Enable the WebView2 accessibility tree for deterministic UI Automation.
    if (-not $NativeOnly) {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "$previousBrowserArguments --force-renderer-accessibility"
    }
    $application = Start-Process -FilePath $Executable -PassThru
    $handle = Wait-Until {
        $application.Refresh()
        if ($application.HasExited) { throw 'Application exited before opening its window.' }
        if ($application.MainWindowHandle -ne [IntPtr]::Zero) { $application.MainWindowHandle }
    } 'main window'
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
    if (-not $NativeOnly) {
        Write-Output 'Native window opened; waiting for the IPC readiness indicator.'
        $null = Wait-Until { Find-Name $root $readyText } 'Rust IPC readiness'
        Write-Output 'Rust IPC ready; checking single-instance behavior.'
    } else {
        Start-Sleep -Seconds 3
        Write-Output 'Native-only test: IPC/UI assertions are intentionally not performed.'
    }

    $duplicate = Start-Process -FilePath $Executable -PassThru
    if (-not $duplicate.WaitForExit(10000)) {
        $duplicate.Kill()
        throw 'Duplicate launch did not exit.'
    }
    if ($duplicate.ExitCode -ne 0) { throw 'Duplicate launch returned a nonzero exit code.' }
    $application.Refresh()
    if ($application.HasExited) { throw 'Original instance exited after duplicate launch.' }

    if (-not $NativeOnly) {
        $newSession = Find-Name $root $newSessionText
        if (-not $newSession) { throw 'New session button is missing.' }
        $newSession.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        $input = Wait-Until {
            $element = Find-Name $root $promptText
            if ($element -and $element.Current.IsEnabled) { return $element }
        } 'session composer'
        $input.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('Phase 1 workbench planning smoke test.')
        $previousId = 0L
        $existing = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )
        foreach ($element in $existing) {
            if ($element.Current.Name -match '^#(\d+)') {
                $previousId = [Math]::Max($previousId, [long]$Matches[1])
            }
        }
        $button = Wait-Until {
            $element = Find-Name $root $startText
            if ($element -and $element.Current.IsEnabled) { return $element }
        } 'enabled Send button'
        $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        $null = Wait-Until {
            $elements = $root.FindAll(
                [System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition
            )
            foreach ($element in $elements) {
                $name = $element.Current.Name
                if ($name -match "^$([regex]::Escape($replyText)) (\d+)$" -and [long]$Matches[1] -gt $previousId -and (Find-Name $element $completedText)) {
                    return $true
                }
            }
        } 'completed model response' 120
    }

    $processes = @(Get-CimInstance Win32_Process)
    $ids = @($application.Id)
    do {
        $children = @($processes | Where-Object { $ids -contains [int]$_.ParentProcessId -and $ids -notcontains [int]$_.ProcessId })
        $ids += @($children | ForEach-Object { [int]$_.ProcessId })
    } while ($children.Count -gt 0)
    $workingSet = ($processes | Where-Object { $ids -contains [int]$_.ProcessId } | Measure-Object -Property WorkingSetSize -Sum).Sum
    Write-Output ('Sampled working set (app + WebView2 children): {0:N1} MiB' -f ($workingSet / 1MB))

    $null = $application.CloseMainWindow()
    if (-not $application.WaitForExit(15000)) { throw 'Graceful shutdown timed out.' }
    if ($application.ExitCode -ne 0) { throw 'Application reported a shutdown failure.' }
    if ($NativeOnly) {
        Write-Output 'PASS: native window, single instance, graceful shutdown. IPC/UI not verified.'
    } else {
        Write-Output 'PASS: model response, real IPC stream, single instance, graceful shutdown.'
    }
} catch {
    if ($application -and -not $application.HasExited -and $root) {
        $elements = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )
        Write-Output "Accessible nodes: $($elements.Count)"
        # Do not dump the UI tree: Settings may contain credentials.
    }
    throw
} finally {
    if ($application -and -not $application.HasExited) {
        $null = $application.CloseMainWindow()
        if (-not $application.WaitForExit(15000)) { $application.Kill() }
    }
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousBrowserArguments
}




