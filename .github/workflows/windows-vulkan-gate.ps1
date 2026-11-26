$ErrorActionPreference = 'Stop'

# Hosted Windows runners have no physical GPU. Use an isolated Vulkan ICD so
# the renderer tests run without relying on the D3D12 software driver.
$runtime = Join-Path ([System.IO.Path]::GetTempPath()) 'texturecraft-ci-vulkan'
New-Item -ItemType Directory -Path $runtime -Force | Out-Null

function Get-VerifiedArchive($uri, $name, $sha256) {
    $archive = Join-Path $runtime $name
    Invoke-WebRequest -Uri $uri -OutFile $archive -TimeoutSec 120
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $sha256) {
        throw "Unexpected SHA-256 for $name"
    }
    return $archive
}

$mesa = Get-VerifiedArchive `
    'https://github.com/pal1000/mesa-dist-win/releases/download/26.3.0/mesa3d-26.3.0-release-msvc.7z' `
    'mesa.7z' 'C505FCC77E8F63A09D7DDAC086591D2F542C0698D37FC9E3094685A34D0D9991'
$loader = Get-VerifiedArchive `
    'https://sdk.lunarg.com/sdk/download/1.4.363.0/windows/VulkanRT-X64-1.4.363.0-Components.zip' `
    'vulkan.zip' 'A25A927AA8B9F0371048F1861CF88AC3B9BC9B1FB332C42D897C8AB32695769A'

# The shared action starts this script from Git Bash, whose tar treats C: as a remote host.
$system = [System.Environment]::SystemDirectory
$tar = Join-Path $system 'tar.exe'
& $tar -xf $mesa -C $runtime x64/lvp_icd.x86_64.json x64/vulkan_lvp.dll
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$loaderRoot = 'VulkanRT-X64-1.4.363.0-Components/x64'
& $tar -xf $loader -C $runtime "$loaderRoot/vulkan-1.dll" "$loaderRoot/vulkaninfo.exe"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$loaderBin = Join-Path $runtime $loaderRoot
$env:PATH = "$loaderBin;$env:PATH"
$env:VK_DRIVER_FILES = Join-Path $runtime 'x64/lvp_icd.x86_64.json'
if ($env:RUNNER_ENVIRONMENT -eq 'github-hosted') {
    # The loader can ignore environment-based driver discovery for elevated processes.
    $drivers = 'HKLM:\SOFTWARE\Khronos\Vulkan\Drivers'
    New-Item -Path $drivers -Force | Out-Null
    New-ItemProperty -LiteralPath $drivers -Name $env:VK_DRIVER_FILES -PropertyType DWord -Value 0 -Force | Out-Null
    $systemLoader = Join-Path $system 'vulkan-1.dll'
    if (!(Test-Path -LiteralPath $systemLoader)) {
        Copy-Item -LiteralPath (Join-Path $loaderBin 'vulkan-1.dll') -Destination $systemLoader
    }
}
$env:WGPU_BACKEND = 'vulkan'
$env:TC_CI_LAVAPIPE = '1'
# Keep software devices from contending for all runner cores at once.
$env:RUST_TEST_THREADS = '2'
$env:LP_NUM_THREADS = '2'
# LLVM's optimisation passes cost more than the tests' shaders then run
$env:GALLIVM_PERF = 'nopt'

Write-Host 'Windows CI using Mesa Lavapipe 26.3.0 with Vulkan loader 1.4.363.0'
$summary = & (Join-Path $loaderBin 'vulkaninfo.exe') --summary
if ($LASTEXITCODE -ne 0 -or !($summary -match 'deviceName\s*=\s*llvmpipe')) {
    $summary | Write-Host
    throw 'Vulkan preflight did not find the Lavapipe adapter'
}
$summary | Select-String 'deviceType|deviceName|driverInfo' | ForEach-Object { Write-Host $_.Line }
cargo xtask ci
exit $LASTEXITCODE
