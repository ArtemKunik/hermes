# WARNING: Do NOT run this script when ccterm is running.
# ccterm (ccTermAppArtem scheduled task) owns port 38080 and manages hermes as a subprocess.
# Running this script while ccterm is running will conflict and prevent ccterm from starting.
#
# This script is only for standalone hermes testing (no ccterm).
# The normal MCP server for agents is ccterm at http://127.0.0.1:38080/api/mcp.

$ccterm = Get-Process ccterm -ErrorAction SilentlyContinue
if ($ccterm) {
    Write-Error "[hermes] ccterm (PID $($ccterm.Id)) is already running on port 38080. Do NOT start standalone hermes — ccterm manages hermes internally. Connect agents to http://127.0.0.1:38080/api/mcp."
    exit 1
}

$listeners = netstat -ano 2>$null | Select-String ":38080 .+LISTENING"
if ($listeners) {
    Write-Error "[hermes] Port 38080 is already in use. Check who is listening before starting standalone hermes."
    exit 1
}

$env:HERMES_PROJECT_ROOT             = "D:\source\hermes"
$env:HERMES_PROJECTS                 = "D:\source\Lunapark;D:\source\SmartPositionAssistant;D:\source\hermes"
$env:HERMES_AUTO_INDEX_INTERVAL_SECS = "300"

$exe = "D:\source\hermes\target\release\Hermes.exe"

$proc = Start-Process -FilePath $exe `
              -ArgumentList "--http", "38080" `
              -NoNewWindow `
              -PassThru
Write-Host "[hermes] started PID $($proc.Id) → http://localhost:38080/api/mcp"
