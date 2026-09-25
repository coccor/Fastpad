[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Renders assets/icons/material/svg/*.svg into assets/icons/material/icons.bin and records the
# sources' hash (icon sets spec §5.1). The renderer is the Markdown preview's Direct2D SVG code,
# run from an ignored test so it never ships.
Push-Location (Join-Path $PSScriptRoot "..")
try {
    cargo test --lib window::icon_sets::generate::generate_material_icons -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw "the icon generator failed" }
} finally {
    Pop-Location
}
