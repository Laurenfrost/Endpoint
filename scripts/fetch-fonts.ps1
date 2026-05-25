# 下载阶段四所需字体资源。字体文件已加入 .gitignore，首次开发或 CI 时需运行本脚本。
# 用法：powershell -ExecutionPolicy Bypass -File scripts/fetch-fonts.ps1

$fontsDir = "$PSScriptRoot\..\src-tauri\resources\fonts"
New-Item -ItemType Directory -Force $fontsDir | Out-Null

# $lxgwPath = "$fontsDir\LXGWWenKai-Regular.ttf"
# if (Test-Path $lxgwPath) {
#     Write-Host "LXGWWenKai-Regular.ttf already exists. Skip."
# } else {
#     Write-Host "Downloading..."
#     Invoke-WebRequest `
#         -Uri "https://github.com/lxgw/LxgwWenKai/releases/download/v1.522/LXGWWenKai-Regular.ttf" `
#         -OutFile $lxgwPath
#     Write-Host "Complete: $lxgwPath"
# }

$sansPath = "$fontsDir\SourceHanSerifCN-Regular.otf"
if (Test-Path $sansPath) {
    Write-Host "SourceHanSerifCN-Regular.otf already exists. Skip."
} else {
    Write-Host "Downloading..."
    Invoke-WebRequest `
        -Uri "https://github.com/adobe-fonts/source-han-serif/releases/latest/download/SourceHanSerifCN.zip" `
        -OutFile "$fontsDir\SourceHanSerifCN.zip"
    Write-Host "Please unzip it manually."
}
