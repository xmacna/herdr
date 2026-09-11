$ErrorActionPreference = 'Stop'

function Assert-True([bool] $condition, [string] $message) {
    if (-not $condition) { throw "ASSERTION FAILED: $message" }
}

function Assert-Equal($actual, $expected, [string] $message) {
    if ($actual -ne $expected) {
        throw "ASSERTION FAILED: $message (actual='$actual', expected='$expected')"
    }
}

function Get-Selection([object] $manifest, [switch] $WithoutWhatsApp) {
    $selected = [System.Collections.Generic.List[string]]::new()
    foreach ($id in $manifest.selection.core) { $selected.Add([string]$id) }
    if (-not $WithoutWhatsApp) { $selected.Add('wacli-generic') }
    return $selected
}

$packageRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $packageRoot 'skills/manifest.json'
$manifest = Get-Content -Raw -Encoding UTF8 $manifestPath | ConvertFrom-Json
Assert-Equal $manifest.schema_version 1 'manifest schema'
Assert-True ($manifest.selection.core -contains 'herdr-runtime') 'Herdr is core'
Assert-True ($manifest.selection.core -contains 'profile') 'profile is core'
Assert-Equal $manifest.selection.whatsapp.default_selected $true 'WhatsApp default selection'
Assert-Equal $manifest.selection.whatsapp.opt_out_flag '--without-whatsapp' 'WhatsApp opt-out flag'
Assert-True ((Get-Selection $manifest) -contains 'wacli-generic') 'default includes WACLI'
Assert-True ((Get-Selection $manifest -WithoutWhatsApp) -notcontains 'wacli-generic') 'opt-out removes WACLI'
Assert-True ($manifest.excluded.id -contains 'xmacna-specific') 'excluded private category is explicit'

$allowedTokens = @('{{ARGOS_PROFILE_RESPONSE}}', '{{ARGOS_PROFILE_PATH}}', '{{ARGOS_WORKSPACE}}', '{{ARGOS_LOCALE}}')
foreach ($locale in @('pt', 'en')) {
    foreach ($kind in @('SOUL.md', 'AGENTS.md', 'CLAUDE.md')) {
        $path = Join-Path $packageRoot "templates/$locale/$kind"
        Assert-True (Test-Path $path) "template exists: $locale/$kind"
        $content = Get-Content -Raw -Encoding UTF8 $path
        foreach ($line in ($content -split "`r?`n" | Where-Object { $_ -like '*{{*' })) {
            foreach ($token in ([regex]::Matches($line, '\{\{[^}]+\}\}') | ForEach-Object Value)) {
                Assert-True ($allowedTokens -contains $token) "known token $token"
            }
        }
    }
}

$fixture = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot 'fixtures/profile-response.json') | ConvertFrom-Json
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "argos-onboarding-contract-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
try {
    $profilePath = Join-Path $tempRoot 'SOUL.md'
    $agentsPath = Join-Path $tempRoot 'AGENTS.md'
    $claudePath = Join-Path $tempRoot 'CLAUDE.md'
    $soul = Get-Content -Raw -Encoding UTF8 (Join-Path $packageRoot 'templates/pt/SOUL.md')
    $soul = $soul.Replace('{{ARGOS_PROFILE_RESPONSE}}', [string]$fixture.response)
    $soul = $soul.Replace('{{ARGOS_PROFILE_PATH}}', $profilePath)
    [IO.File]::WriteAllText($profilePath, $soul, [Text.UTF8Encoding]::new($false))
    foreach ($kind in @('AGENTS.md', 'CLAUDE.md')) {
        $template = Get-Content -Raw -Encoding UTF8 (Join-Path $packageRoot "templates/pt/$kind")
        $template = $template.Replace('{{ARGOS_PROFILE_PATH}}', $profilePath)
        $template = $template.Replace('{{ARGOS_WORKSPACE}}', $tempRoot)
        $target = if ($kind -eq 'AGENTS.md') { $agentsPath } else { $claudePath }
        [IO.File]::WriteAllText($target, $template, [Text.UTF8Encoding]::new($false))
    }
    $profile = Get-Content -Raw -Encoding UTF8 $profilePath
    $accented = 'a' + [char]0x00e7 + [char]0x00e3 + 'o'
    Assert-True $profile.Contains($accented) 'profile preserves accented literal response'
    Assert-True $profile.Contains('"literais"') 'profile preserves quoted literal response'
    $agents = Get-Content -Raw -Encoding UTF8 $agentsPath
    $claude = Get-Content -Raw -Encoding UTF8 $claudePath
    Assert-True $agents.Contains($profilePath) 'AGENTS references canonical profile'
    Assert-True $claude.Contains($profilePath) 'CLAUDE references canonical profile'
    Assert-True ($agents -notmatch 'xmacna|5531|labs\.') 'AGENTS has no private fixture data'
    Assert-True ($claude -notmatch 'xmacna|5531|labs\.') 'CLAUDE has no private fixture data'

    $existing = Join-Path $tempRoot 'existing.md'
    [IO.File]::WriteAllText($existing, 'user instructions', [Text.UTF8Encoding]::new($false))
    $backup = "$existing.bak"
    Copy-Item -LiteralPath $existing -Destination $backup
    Assert-True ((Get-Content -Raw -Encoding UTF8 $existing) -eq 'user instructions') 'existing profile is preserved'
    Assert-True (Test-Path $backup) 'backup exists before managed update'
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force
}

foreach ($fixturePath in Get-ChildItem (Join-Path $PSScriptRoot 'fixtures/cli-states') -Filter '*.json') {
    $state = Get-Content -Raw -Encoding UTF8 -LiteralPath $fixturePath.FullName | ConvertFrom-Json
    Assert-True ($null -ne $state.codex -and $null -ne $state.claude) "separate provider state: $($state.case)"
    Assert-True ($state.case -ne 'network-failure' -or $state.network -eq 'unavailable') 'network failure is explicit'
}

Write-Output 'onboarding contract tests: PASS (manifest, opt-out, templates, literal profile, backup, provider fixtures)'
