[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$tauri = Join-Path $PSScriptRoot '..\src-tauri'
$assets = Join-Path $tauri 'offline'
$dependencies = Join-Path $tauri 'target\offline-deps'
New-Item -ItemType Directory -Force -Path $assets, $dependencies | Out-Null

function Get-Download($url, $path) {
    curl.exe --fail --location --silent --show-error --retry 3 --retry-all-errors --output $path $url
    if ($LASTEXITCODE -ne 0) { throw "下载失败：$url" }
}

$model = Join-Path $assets 'ggml-base-q5_1.bin'
$modelHash = '422F1AE452ADE6F30A004D7E5C6A43195E4433BC370BF23FAC9CC591F01A8898'
if (-not (Test-Path -LiteralPath $model) -or (Get-FileHash -LiteralPath $model).Hash -ne $modelHash) {
    $partial = "$model.part"
    Get-Download 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin' $partial
    if ((Get-FileHash -LiteralPath $partial).Hash -ne $modelHash) { throw 'Whisper 模型校验失败' }
    Move-Item -LiteralPath $partial -Destination $model -Force
}

$source = Join-Path $dependencies 'whisper.cpp-1.8.3'
if (-not (Test-Path -LiteralPath (Join-Path $source 'CMakeLists.txt'))) {
    $archive = Join-Path $dependencies 'whisper-v1.8.3.tar.gz'
    Get-Download 'https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/refs/tags/v1.8.3' $archive
    tar.exe -xzf $archive -C $dependencies
    if ($LASTEXITCODE -ne 0) { throw '无法解压 whisper.cpp 源码' }
}

$cmake = Get-Command cmake.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $cmake) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vs = & $vswhere -latest -products '*' -property installationPath
    $cmake = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
}

$build = Join-Path $dependencies 'build'
& $cmake -S (Join-Path $tauri 'inference') -B $build -A x64 "-DWHISPER_SOURCE=$source"
if ($LASTEXITCODE -ne 0) { throw '无法配置离线识别程序' }
& $cmake --build $build --config Release --target voicetag-offline --parallel
if ($LASTEXITCODE -ne 0) { throw '无法编译离线识别程序' }
Copy-Item -LiteralPath (Join-Path $build 'Release\voicetag-offline.exe') -Destination $assets -Force
Copy-Item -LiteralPath (Join-Path $source 'LICENSE') -Destination (Join-Path $assets 'whisper.cpp-LICENSE.txt') -Force
Get-Download 'https://raw.githubusercontent.com/openai/whisper/main/LICENSE' (Join-Path $assets 'whisper-LICENSE.txt')
