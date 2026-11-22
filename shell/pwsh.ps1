# weft shell integration for PowerShell (pwsh 7 and Windows PowerShell 5.1,
# local or remote): prompt and command marks (OSC 133) and the current
# directory (OSC 7). Passed on the command line locally (-EncodedCommand) and
# through the WEFT_PS environment variable on a remote host.

[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$env:WEFT = '1'
Remove-Item Env:WEFT_PS -ErrorAction SilentlyContinue
$global:__weft_host = if ($env:COMPUTERNAME) { $env:COMPUTERNAME } else { [System.Net.Dns]::GetHostName() }
$global:__weft_prompt = $function:prompt
function global:prompt {
    $ok = $global:?
    $e = [char]27; $b = [char]7
    $code = if ($ok) { 0 } elseif ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 }
    $loc = Get-Location
    $osc7 = ''
    if ($loc.Provider.Name -eq 'FileSystem') {
        $p = $loc.ProviderPath -replace '\\', '/'
        if (-not $p.StartsWith('/')) { $p = '/' + $p }
        $osc7 = "$e]7;file://$global:__weft_host$p$b"
    }
    # End the block now: the user's own prompt (starship, oh-my-posh) can take
    # 50-150 ms and must not count as the command's duration.
    [Console]::Write("$e]133;D;$code$b$osc7")
    $null = __weft_keys
    "$e]133;A$b" + (& $global:__weft_prompt) + "$e]133;B$b"
}
# weft's keys in PSReadLine: Enter says a command starts (133;C), and
# Ctrl+Alt+Shift+F12 empties the line in any edit mode (Windows', Emacs',
# Vi's insert and command: back into insert). Bound again at each prompt: a
# change of edit mode (Set-PSReadLineOption -EditMode) puts back the mode's
# own keys only.
function global:__weft_keys {
    if (-not (Get-Module PSReadLine)) { return $false }
    try {
        Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
            [Console]::Write("$([char]27)]133;C$([char]7)")
        }
        $clear = {
            [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
            if ((Get-PSReadLineOption).EditMode -eq 'Vi') { [Microsoft.PowerShell.PSConsoleReadLine]::ViInsertMode() }
        }
        Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+Shift+F12' -ScriptBlock $clear
        if ((Get-PSReadLineOption).EditMode -eq 'Vi') { Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+Shift+F12' -ViMode Command -ScriptBlock $clear }
        return $true
    } catch { return $false }
}
if (__weft_keys) { [Console]::Write("$([char]27)]7777;{`"t`":`"clear-key`"}$([char]7)") }
