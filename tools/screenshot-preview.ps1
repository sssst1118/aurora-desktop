# Aurora README 截图渲染脚本
# 用法: pwsh tools/screenshot-preview.ps1 [输出目录]
# 依赖: 本机已安装 Microsoft Edge(或 Chrome);无需安装 Aurora。
# 原理: 用无头浏览器打开 docs/aurora-v02-preview.html 的深链状态
#       (?shot=panel&view=...&skin=...),截图为 PNG,供 README 引用。
# 输出: 默认 docs/screenshots/ 下的 aurora-*.png(与 README 相对路径一致)。
# 注意: 本文件必须保持 CRLF 换行——PowerShell 5.1 解析 LF 文件时反引号续行会失效,
#       故代码内不使用反引号续行(git 配置了 autocrlf,checkout 时自动转 CRLF)。

$ErrorActionPreference = "Continue"  # 不用 Stop:PS 5.1 下 native stderr 任意一行(Edge 无害 ERROR)都会终止脚本
$root = Split-Path -Parent $PSScriptRoot
$preview = Join-Path $root "docs\aurora-v02-preview.html"
$outDir = if ($args.Count -gt 0) { $args[0] } else { Join-Path $root "docs\screenshots" }
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

# 找浏览器(Edge 优先,Chrome 兜底);注意 ${env:ProgramFiles(x86)} 必须加大括号
$browser = $null
foreach ($cand in @(
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
  "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe"
)) {
  if (Test-Path $cand) { $browser = $cand; break }
}
if (-not $browser) { Write-Error "未找到 Edge/Chrome,无法渲染截图"; exit 1 }

# 每次截图用独立临时 profile(防复用用户正在跑的 Edge 实例导致 --screenshot 静默不生效)
$profileDir = Join-Path $env:TEMP ("aurora-shot-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $profileDir | Out-Null
$errLog = Join-Path $env:TEMP "aurora-shot-err.log"

# 截图清单: 文件名 -> @{ 参数; 窗口宽; 窗口高 }
$shots = [ordered]@{
  "aurora-island.png"          = @{ q = "";                          w = 900; h = 220 }
  "aurora-panel-drawer.png"    = @{ q = "?shot=panel&view=drawer";   w = 900; h = 620 }
  "aurora-panel-search.png"    = @{ q = "?shot=panel&view=search&q=%E8%AE%B0"; w = 900; h = 620 }
  "aurora-panel-clip.png"      = @{ q = "?shot=panel&view=clip";     w = 900; h = 620 }
  "aurora-panel-ai.png"        = @{ q = "?shot=panel&view=ai";       w = 900; h = 620 }
  "aurora-panel-settings.png"  = @{ q = "?shot=panel&view=settings"; w = 900; h = 620 }
  "aurora-skin-midnight.png"   = @{ q = "?shot=panel&view=settings&skin=midnight"; w = 900; h = 620 }
  "aurora-skin-dawn.png"       = @{ q = "?shot=panel&view=settings&skin=dawn";     w = 900; h = 620 }
  "aurora-skin-verdant.png"    = @{ q = "?shot=panel&view=settings&skin=verdant";  w = 900; h = 620 }
}

foreach ($name in $shots.Keys) {
  $s = $shots[$name]
  $url = "file:///" + ($preview -replace "\\", "/") + $s.q
  $out = Join-Path $outDir $name
  # 参数放数组避免行尾反引号续行(PS 5.1 下 LF 文件续行失效);stderr 落文件而非丢弃:
  # ErrorActionPreference=Stop 时 native stderr 任意一行都会终止脚本,而 Edge headless 总会打无害的 ERROR 行
  $args = @(
    "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=2",
    "--virtual-time-budget=3000", "--window-size=$($s.w),$($s.h)",
    "--user-data-dir=$profileDir", "--screenshot=$out", $url
  )
  & $browser @args 2> $errLog | Out-Null
  if (-not (Test-Path $out)) { Write-Error "截图失败: $name(Edge 可能未按预期工作,详见 $errLog)"; exit 1 }
  Write-Output "OK $name ($((Get-Item $out).Length) bytes)"
}
Remove-Item -Recurse -Force $profileDir -ErrorAction SilentlyContinue
Write-Output "完成: $outDir"
