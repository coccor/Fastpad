[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$SourceRoot = Join-Path $RepositoryRoot "native\src"
$ScintillaInterface = Join-Path $SourceRoot "scintilla\include\Scintilla.iface"
$LexillaInterface = Join-Path $SourceRoot "lexilla\include\LexicalStyles.iface"
$OutputPath = Join-Path $RepositoryRoot "src\editor\scintilla_constants.rs"

& (Join-Path $PSScriptRoot "fetch-native.ps1") -VerifyOnly
if (-not $?) {
    throw "Native source verification failed"
}

$RequiredNames = @(
    "SCI_GETDIRECTFUNCTION", "SCI_GETDIRECTPOINTER", "SCI_SETCODEPAGE", "SCI_SETTEXT",
    "SCI_GETTEXT", "SCI_GETTEXTLENGTH", "SCI_GETLENGTH", "SCI_SETUNDOCOLLECTION",
    "SCI_EMPTYUNDOBUFFER", "SCI_SETSAVEPOINT", "SCI_BEGINUNDOACTION", "SCI_ENDUNDOACTION",
    "SCI_CREATEDOCUMENT", "SCI_ADDREFDOCUMENT", "SCI_RELEASEDOCUMENT", "SCI_GETDOCPOINTER",
    "SCI_SETDOCPOINTER", "SCI_UNDO", "SCI_REDO", "SCI_CANUNDO", "SCI_CANREDO", "SCI_CUT",
    "SCI_COPY", "SCI_PASTE", "SCI_GETSELECTIONSTART", "SCI_GETSELECTIONEND", "SCI_SETSEL",
    "SCI_LINEFROMPOSITION", "SCI_GETCOLUMN", "SCI_POSITIONFROMLINE", "SCI_GETLINEENDPOSITION",
    "SCI_SETTARGETRANGE", "SCI_SEARCHINTARGET", "SCI_REPLACETARGET", "SCI_SETSEARCHFLAGS",
    "SCI_SETILEXER", "SCI_STYLESETFORE", "SCI_STYLESETBACK", "SCI_STYLESETFONT", "SCI_STYLESETBOLD",
    "SCI_STYLESETSIZEFRACTIONAL", "SCI_STYLECLEARALL", "SCI_SETMARGINTYPEN", "SCI_SETMARGINWIDTHN",
    "SCI_SETWRAPMODE", "SCI_SETTABWIDTH", "SCI_GETMODIFY", "SC_CP_UTF8", "SCN_SAVEPOINTREACHED",
    "SCN_SAVEPOINTLEFT", "SCN_MODIFIED", "SC_MOD_INSERTTEXT", "SC_MOD_DELETETEXT", "SC_WRAP_NONE",
    "SC_WRAP_WORD", "SCI_SETKEYWORDS", "SCI_SETPROPERTY", "SCI_STYLESETITALIC", "SCI_GETSTYLEAT",
    "STYLE_MAX", "SCI_GOTOLINE", "SCI_GETSELTEXT", "SCFIND_NONE", "SCFIND_WHOLEWORD", "SCFIND_MATCHCASE", "SCI_SCROLLCARET",
    "STYLE_DEFAULT", "SCI_GETTABWIDTH", "SCI_SETCARETFORE", "SCI_SETSCROLLWIDTH",
    "SCI_SETSCROLLWIDTHTRACKING", "SCI_SETELEMENTCOLOUR", "SCI_RESETELEMENTCOLOUR",
    "SC_ELEMENT_SELECTION_BACK", "SC_ELEMENT_SELECTION_TEXT",
    "SC_ELEMENT_SELECTION_INACTIVE_BACK", "SC_ELEMENT_SELECTION_INACTIVE_TEXT",
    "SC_ELEMENT_CARET_LINE_BACK", "SCI_GETFIRSTVISIBLELINE",
    "SCI_SETFIRSTVISIBLELINE", "SCI_SETMARGINLEFT", "SCI_SETMARGINRIGHT",
    "SCI_GETLINECOUNT", "SCI_TEXTWIDTH", "SCI_GETMARGINWIDTHN", "SCI_STYLEGETBACK",
    "SC_MARGIN_NUMBER", "STYLE_LINENUMBER", "SCI_GETCURRENTPOS", "SCI_COUNTCHARACTERS",
    "SCN_UPDATEUI", "SCI_GETZOOM", "SCI_SETZOOM", "SCI_ZOOMIN", "SCI_ZOOMOUT", "SCN_ZOOM",
    "SCI_GETRANGEPOINTER", "SCI_DOCLINEFROMVISIBLE",
    "SCI_VISIBLEFROMDOCLINE", "SC_UPDATE_V_SCROLL", "SCI_GOTOPOS", "SCI_DOCUMENTEND",
    "SCI_GETANCHOR", "SCI_GETLINE", "SCI_LINELENGTH",
    "SCI_GETTARGETEND", "SCI_GETCHARACTERPOINTER", "SCI_GETCODEPAGE",
    "SCI_GETXOFFSET", "SCI_SETXOFFSET", "SCI_GOTOLINE", "SCN_FOCUSIN",
    "SCI_SETUSETABS", "SCI_GETUSETABS", "SCI_SETVIEWWS", "SCI_GETVIEWWS", "SCWS_INVISIBLE",
    "SCWS_VISIBLEALWAYS", "SCI_GETELEMENTISSET",
    "SC_MARGIN_SYMBOL", "SC_MASK_FOLDERS", "SCI_SETMARGINMASKN", "SCI_SETMARGINSENSITIVEN",
    "SCI_MARKERDEFINE", "SCI_MARKERSETFORE", "SCI_MARKERSETBACK", "SCI_MARKERSETBACKSELECTED",
    "SC_MARK_VLINE", "SC_MARK_LCORNER", "SC_MARK_TCORNER", "SC_MARK_BOXPLUS",
    "SC_MARK_BOXPLUSCONNECTED", "SC_MARK_BOXMINUS", "SC_MARK_BOXMINUSCONNECTED",
    "SC_MARKNUM_FOLDEREND", "SC_MARKNUM_FOLDEROPENMID", "SC_MARKNUM_FOLDERMIDTAIL",
    "SC_MARKNUM_FOLDERTAIL", "SC_MARKNUM_FOLDERSUB", "SC_MARKNUM_FOLDER",
    "SC_MARKNUM_FOLDEROPEN", "SCI_SETFOLDMARGINCOLOUR", "SCI_SETFOLDMARGINHICOLOUR",
    "SCI_SETAUTOMATICFOLD", "SC_AUTOMATICFOLD_SHOW", "SC_AUTOMATICFOLD_CLICK",
    "SC_AUTOMATICFOLD_CHANGE", "SCI_SETFOLDFLAGS", "SC_FOLDFLAG_LINEAFTER_CONTRACTED",
    "SCI_FOLDALL", "SC_FOLDACTION_CONTRACT", "SC_FOLDACTION_EXPAND",
    "SCI_ASSIGNCMDKEY", "SCI_CLEARCMDKEY", "SCK_UP", "SCK_DOWN", "SCK_LEFT", "SCK_RIGHT",
    "SCMOD_SHIFT", "SCMOD_CTRL", "SCMOD_ALT", "SCI_LINEUPRECTEXTEND", "SCI_LINEDOWNRECTEXTEND",
    "SCI_CHARLEFTRECTEXTEND", "SCI_CHARRIGHTRECTEXTEND", "SCI_SETMULTIPLESELECTION",
    "SCI_SETADDITIONALSELECTIONTYPING", "SCI_SETMULTIPASTE", "SC_MULTIPASTE_EACH",
    "SCI_MOVESELECTEDLINESUP", "SCI_MOVESELECTEDLINESDOWN", "SCI_GETSELECTIONS",
    "SCI_GETMAINSELECTION", "SCI_SETMAINSELECTION", "SCI_GETSELECTIONNSTART",
    "SCI_GETSELECTIONNEND", "SCI_GETSELECTIONNCARET", "SCI_GETSELECTIONNANCHOR",
    "SCI_ADDSELECTION", "SCI_SETSELECTION", "SCI_MULTIPLESELECTADDNEXT",
    "SCI_MULTIPLESELECTADDEACH", "SCI_TARGETWHOLEDOCUMENT", "SCI_COPYALLOWLINE",
    "SCI_CUTALLOWLINE", "SCI_GETLINEINDENTATION", "SCI_SETLINEINDENTATION",
    "SCI_GETLINEINDENTPOSITION", "SCI_GETINDENT", "SCI_GETEOLMODE", "SC_EOL_CR", "SC_EOL_LF",
    "SCI_FINDCOLUMN", "SCI_POSITIONFROMPOINT", "SCI_POINTXFROMPOSITION", "SCI_POINTYFROMPOSITION"
)

# Every style of the lexers FastPad maps (src/languages/*).
$RequiredPrefixes = @(
    "SCE_JSON_", "SCE_MARKDOWN_", "SCE_H_", "SCE_HJ_", "SCE_CSS_", "SCE_C_", "SCE_YAML_",
    "SCE_TOML_", "SCE_PROPS_", "SCE_POWERSHELL_", "SCE_SH_", "SCE_BAT_", "SCE_P_", "SCE_RUST_",
    "SCE_SQL_"
)

function Add-Definitions {
    param(
        [Parameter(Mandatory = $true)] [string]$InterfacePath,
        [Parameter(Mandatory = $true)] [hashtable]$Definitions
    )

    foreach ($line in Get-Content -LiteralPath $InterfacePath) {
        if ($line -match '^(fun|get|set|evt)\s+\S+\s+([A-Za-z0-9_]+)=([^\s(]+)') {
            $prefix = if ($Matches[1] -eq "evt") { "SCN_" } else { "SCI_" }
            $Definitions["$prefix$($Matches[2].ToUpperInvariant())"] = $Matches[3]
        }
        elseif ($line -match '^val\s+([A-Z0-9_]+)=([^\s]+)') {
            $Definitions[$Matches[1]] = $Matches[2]
        }
    }
}

$Definitions = @{}
Add-Definitions -InterfacePath $ScintillaInterface -Definitions $Definitions
Add-Definitions -InterfacePath $LexillaInterface -Definitions $Definitions

$MissingNames = @($RequiredNames | Where-Object { -not $Definitions.ContainsKey($_) })
if ($MissingNames.Count -ne 0) {
    throw "Required Scintilla constants were not found: $($MissingNames -join ', ')"
}

$Names = [System.Collections.Generic.HashSet[string]]::new([string[]]$RequiredNames)
foreach ($prefix in $RequiredPrefixes) {
    $matched = @($Definitions.Keys | Where-Object { $_.StartsWith($prefix, [System.StringComparison]::Ordinal) })
    if ($matched.Count -eq 0) {
        throw "No Lexilla styles found for prefix $prefix"
    }
    foreach ($name in $matched) { [void]$Names.Add($name) }
}

$Lines = @(
    "// Generated by tools/generate-scintilla-constants.ps1 from Scintilla 5.6.6 and Lexilla 5.5.3.",
    "// Do not edit by hand.",
    ""
)
foreach ($name in $Names | Sort-Object) {
    $Lines += "pub const $($name): u32 = $($Definitions[$name]);"
}

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutputPath) | Out-Null
[System.IO.File]::WriteAllLines($OutputPath, $Lines, [System.Text.UTF8Encoding]::new($false))
