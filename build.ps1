# 构建并安装 mos_灯光混合（EffectCraft 插件）
# 用法：右键“使用 PowerShell 运行”，或在终端执行  .\build.ps1

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$appData = $env:APPDATA
if (-not $appData) { $appData = [Environment]::GetFolderPath('ApplicationData') }

Write-Host '[1/3] 激活 Rust 工具链 ...'
. 'D:\ruanjian\rust-env\activate.ps1'
$env:CARGO_NET_OFFLINE = 'true'

Write-Host '[2/3] 编译 helper ...'
Push-Location (Join-Path $here 'helper')
# MinGW 的 ld 在中文路径下读不到 deps 里的 .o/.rlib（报 No such file or directory，其实文件在），
# 所以项目路径含非 ASCII 字符时，把 target 目录放到纯英文路径下。
$targetDir = Join-Path $here 'helper\target'
$cargoArgs = @('build', '--release')
if ($here -match '[^\x00-\x7F]') {
    $targetDir = Join-Path $env:TEMP 'mos-lightmix-target'
    $cargoArgs += @('--target-dir', $targetDir)
    Write-Host "    路径含中文，target 目录改用 $targetDir"
}
try {
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "编译失败（cargo 退出码 $LASTEXITCODE），没有安装任何文件" }
} finally {
    Pop-Location
}

$exe = Join-Path $targetDir 'release\effectcraft-helper-mos-lightmix.exe'
if (-not (Test-Path $exe)) { throw "编译失败：找不到 $exe" }

Write-Host '[3/4] 安装到 EffectCraft ...'
Get-Process effectcraft-helper-mos-lightmix -ErrorAction SilentlyContinue | ForEach-Object {
    $_.Kill()
    $_.WaitForExit()
}
$dest = Join-Path $appData 'EffectCraft\helpers'
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item $exe (Join-Path $dest 'effectcraft-helper-mos-lightmix.exe') -Force
New-Item -ItemType Directory -Force -Path (Join-Path $here 'dist') | Out-Null
Copy-Item $exe (Join-Path $here 'dist\effectcraft-helper-mos-lightmix.exe') -Force

Write-Host '[4/4] 安装开机自加载脚本 ...'
# EffectCraft 默认不启动外部 helper，靠 Scripts\Startup 里的脚本在每次启动时扫描一次
$startup = Join-Path $appData 'EffectCraft\Scripts\Startup'
New-Item -ItemType Directory -Force -Path $startup | Out-Null
Copy-Item (Join-Path $here 'tools\mos-load-helpers.jsx') (Join-Path $startup 'mos-load-helpers.jsx') -Force

Write-Host ''
Write-Host "完成：$dest\effectcraft-helper-mos-lightmix.exe"
Write-Host '重启 EffectCraft 后，效果列表「MOS」分类下会自动出现 mos_灯光混合。'
Write-Host '（当前已开着的窗口：执行一次 效果 ▸ 扫描效果辅助程序... 即可立刻出现）'
