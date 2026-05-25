# 下载阶段四所需字体资源。字体文件已加入 .gitignore，首次开发或 CI 时需运行本脚本。
# 用法：powershell -ExecutionPolicy Bypass -File scripts/fetch-fonts.ps1

$fontsDir = "$PSScriptRoot\..\src-tauri\resources\fonts"
New-Item -ItemType Directory -Force $fontsDir | Out-Null

# ── 霞鹜文楷 Regular（必须，约 16 MB）──
$lxgwPath = "$fontsDir\LXGWWenKai-Regular.ttf"
if (Test-Path $lxgwPath) {
    Write-Host "已存在: LXGWWenKai-Regular.ttf，跳过。"
} else {
    Write-Host "下载 LXGWWenKai-Regular.ttf ..."
    Invoke-WebRequest `
        -Uri "https://github.com/lxgw/LxgwWenKai/releases/download/v1.522/LXGWWenKai-Regular.ttf" `
        -OutFile $lxgwPath
    Write-Host "完成: $lxgwPath"
}

# ── 思源宋体 CN Regular（可选，约 22 MB，取消注释以下几行以启用）──
# $sansPath = "$fontsDir\SourceHanSerifCN-Regular.otf"
# if (Test-Path $sansPath) {
#     Write-Host "已存在: SourceHanSerifCN-Regular.otf，跳过。"
# } else {
#     Write-Host "下载 SourceHanSerifCN-Regular.otf ..."
#     Invoke-WebRequest `
#         -Uri "https://github.com/adobe-fonts/source-han-serif/releases/latest/download/SourceHanSerifCN.zip" `
#         -OutFile "$fontsDir\SourceHanSerifCN.zip"
#     # 需要解压后复制 Regular 字重
#     Write-Host "请手动从压缩包中提取 SourceHanSerifCN-Regular.otf"
# }

Write-Host "字体资源准备完毕。"
