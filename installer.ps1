<#
  Jarvis — installation, mise à jour et désinstallation depuis les sources publiées.

  Première installation (PowerShell, sans droits administrateur) :
      powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/worganic/jarvis/main/installer.ps1 | iex"

  Le script récupère la dernière version publiée (tag v4.x.y du dépôt de diffusion), installe ce qu'il faut pour
  compiler (Git, Rust, Visual Studio Build Tools, WebView2 : winget), compile Jarvis sur ce PC, copie le programme dans
  %LOCALAPPDATA%\Jarvis\app, supprime les sources récupérées et télécharge les modèles vocaux. Un exe compilé sur le PC
  même n'est pas bloqué par SmartScreen : aucun certificat n'est nécessaire.

  Emplacements (%LOCALAPPDATA%\Jarvis) :
      app\        programme (remplacé à chaque mise à jour ; l'ancien est gardé dans app.precedent\)
      donnees\    tes données (jamais touchées par une mise à jour ni une désinstallation sans ton accord)
      build\      cache de compilation (target\) : les mises à jour suivantes ne recompilent que ce qui a changé

  Ensuite, le script copié dans app\ sert à :
      -Preparer [-Version v4.x.y]      (lancé par Jarvis, qui reste ouvert : récupère et compile la nouvelle version)
      -Appliquer [-SansRelancer]       (lancé par Jarvis qui se ferme : remplace le programme par la version préparée)
      -MiseAJour                       (les deux à la suite, en manuel : demande de fermer Jarvis après la compilation)
      -AjouterModule <id> / -RetirerModule <id>   (interface seule : immédiat ; partie noyau : préparé, appliqué à la relance)
      -Restaurer <archive.jarvis> [-MotDePasse m]
      -LibererPlace                    (vide le cache de compilation)
      -Source <dépôt git> -Version <tag> (essais : clone ce dépôt au lieu de github.com/<Depot>)
      -Desinstaller
#>
[CmdletBinding()]
param(
  [string]$Depot = 'worganic/jarvis',
  # Essais : dépôt Git local (ou autre URL) à cloner à la place de github.com/<Depot> ; avec -Version.
  [string]$Source = '',
  [string]$Version = '',
  [ValidateSet('stable', 'beta')][string]$Canal = '',
  [string[]]$Modules,
  [switch]$MiseAJour,
  [switch]$Preparer,
  [switch]$Appliquer,
  [switch]$SansRelancer,
  [string]$AjouterModule = '',
  [string]$RetirerModule = '',
  [string]$Restaurer = '',
  [string]$MotDePasse = '',
  [switch]$SansModeles,
  [switch]$LibererPlace,
  [switch]$Desinstaller,
  [switch]$Silencieux
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Base = Join-Path $env:LOCALAPPDATA 'Jarvis'
$App = Join-Path $Base 'app'
$Prev = Join-Path $Base 'app.precedent'
$Build = Join-Path $Base 'build'
$Src = Join-Path $Build 'src'
$Target = Join-Path $Build 'target'
$Data = Join-Path $Base 'donnees'
$InstallFile = Join-Path $App 'install.json'
$Log = Join-Path $Base 'installation.log'
# Version compilée qui attend d'être installée (Préparer → Appliquer) : lue aussi par Jarvis (update.rs).
$Ready = Join-Path $Build 'pret.json'
Set-Location $env:TEMP # jamais dans app\, qui est remplacé

# --- Affichage -------------------------------------------------------------------------------------------------------

function Say([string]$text, [string]$color = 'Gray') {
  Write-Host $text -ForegroundColor $color
  try { Add-Content -Path $Log -Value ("[{0}] {1}" -f (Get-Date -Format 's'), $text) -Encoding UTF8 } catch {}
}
function Step([string]$text) { Say ''; Say "== $text" 'Cyan' }
function Fail([string]$text) {
  Say "ÉCHEC : $text" 'Red'
  if (-not $Silencieux) { Read-Host 'Entrée pour fermer' | Out-Null }
  exit 1
}
function Ask([string]$question, [bool]$default = $true) {
  if ($Silencieux) { return $default }
  $hint = if ($default) { '[O/n]' } else { '[o/N]' }
  $a = Read-Host "$question $hint"
  if ([string]::IsNullOrWhiteSpace($a)) { return $default }
  return $a.Trim().ToLower().StartsWith('o')
}

# --- Fiche d'installation ----------------------------------------------------------------------------------------------

function Read-Install {
  if (Test-Path $InstallFile) { return Get-Content $InstallFile -Raw -Encoding UTF8 | ConvertFrom-Json }
  return $null
}
function Write-Install($info) {
  $json = $info | ConvertTo-Json -Depth 5
  [IO.File]::WriteAllText($InstallFile, $json, (New-Object Text.UTF8Encoding $false))
}

# --- Versions publiées -------------------------------------------------------------------------------------------------

function Get-Releases([string]$canal) {
  $tags = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Depot/tags?per_page=100" -Headers @{ 'User-Agent' = 'jarvis-installer' }
  $list = foreach ($t in $tags) {
    if ($t.name -match '^v(\d+)\.(\d+)\.(\d+)(-beta)?$') {
      if ($canal -eq 'stable' -and $Matches[4]) { continue }
      [pscustomobject]@{ Tag = $t.name; Key = [version]("{0}.{1}.{2}" -f $Matches[1], $Matches[2], $Matches[3]); Beta = [bool]$Matches[4] }
    }
  }
  return $list | Sort-Object Key, @{ Expression = { -not $_.Beta } } -Descending
}

function Get-Raw([string]$tag, [string]$path) {
  Invoke-RestMethod -UseBasicParsing -Uri "https://raw.githubusercontent.com/$Depot/$tag/$path" -Headers @{ 'User-Agent' = 'jarvis-installer' }
}

# --- Prérequis ---------------------------------------------------------------------------------------------------------

function Refresh-Path {
  $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + (Join-Path $env:USERPROFILE '.cargo\bin')
}
function Has([string]$cmd) { return [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }
function Winget([string]$id, [string]$extra = '') {
  if (-not (Has 'winget')) { Fail "winget est introuvable (Windows 10 21H2 ou plus récent, « Programme d'installation d'application » du Microsoft Store). Installe $id à la main puis relance." }
  Say "Installation de $id…"
  $cmd = "winget install -e --id $id --silent --accept-package-agreements --accept-source-agreements $extra"
  cmd /c $cmd | Out-Null
  Refresh-Path
}
function Has-BuildTools {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path $vswhere)) { return $false }
  $p = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  return [bool]$p
}
function Has-WebView2 {
  foreach ($k in 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') {
    if (Test-Path $k) { return $true }
  }
  return $false
}

function Ensure-Prerequisites {
  Step 'Outils de compilation'
  Refresh-Path
  $missing = @()
  if (-not (Has 'git')) { $missing += 'Git' }
  if (-not (Has 'cargo')) { $missing += 'Rust' }
  if (-not (Has-BuildTools)) { $missing += 'Visual Studio Build Tools (C++)' }
  if (-not (Has-WebView2)) { $missing += 'WebView2' }
  if ($missing.Count -eq 0) { Say 'Tout est déjà là.' 'Green'; return }
  Say ("À installer : " + ($missing -join ', ') + ". Les Build Tools pèsent plusieurs Go et prennent 10 à 20 minutes.")
  if (-not (Ask 'Les installer maintenant ?')) { Fail 'prérequis manquants.' }
  if ($missing -contains 'Git') { Winget 'Git.Git' }
  if ($missing -contains 'Visual Studio Build Tools (C++)') {
    Winget 'Microsoft.VisualStudio.2022.BuildTools' '--override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
  }
  if ($missing -contains 'Rust') { Winget 'Rustlang.Rustup' }
  if ($missing -contains 'WebView2') { Winget 'Microsoft.EdgeWebView2Runtime' }
  Refresh-Path
  if (Has 'rustup') { & rustup default stable-x86_64-pc-windows-msvc | Out-Null }
  if (-not (Has 'git') -or -not (Has 'cargo') -or -not (Has-BuildTools)) {
    Fail 'un prérequis ne s''est pas installé. Redémarre le PC puis relance ce script.'
  }
}

# --- Jarvis ouvert -----------------------------------------------------------------------------------------------------

function Running { return @(Get-Process -Name 'jarvis', 'jarvis-core', 'jarvis-voice' -ErrorAction SilentlyContinue) }
function Wait-JarvisClosed([int]$seconds = 120) {
  $until = (Get-Date).AddSeconds($seconds)
  while ((Running).Count -gt 0) {
    if ((Get-Date) -gt $until) {
      if (-not (Ask 'Jarvis est encore ouvert. Le fermer de force ?' $true)) { Fail 'Jarvis est ouvert.' }
      Running | Stop-Process -Force
      Start-Sleep -Seconds 2
      return
    }
    Start-Sleep -Milliseconds 500
  }
}
function Start-Jarvis {
  $exe = Join-Path $App 'jarvis.exe'
  if (Test-Path $exe) { Start-Process -FilePath $exe -WorkingDirectory $App }
}

# --- Récupération, choix des modules, compilation ----------------------------------------------------------------------

function Fetch-Sources([string]$tag) {
  Step "Récupération de Jarvis $tag"
  if (Test-Path $Src) { Remove-Item $Src -Recurse -Force }
  New-Item -ItemType Directory -Force $Build | Out-Null
  $url = if ($Source) { $Source } else { "https://github.com/$Depot.git" }
  & git clone --quiet --depth 1 --branch $tag $url $Src 2>&1 | Out-Null
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path (Join-Path $Src 'Cargo.toml'))) { Fail "impossible de récupérer $tag depuis github.com/$Depot." }
}

function Read-Catalogue {
  $f = Join-Path $Src 'modules\catalogue.json'
  if (-not (Test-Path $f)) { Fail 'catalogue des modules absent de cette version.' }
  return (Get-Content $f -Raw -Encoding UTF8 | ConvertFrom-Json).modules
}

# Ajoute les dépendances, garde le socle, retire les ids inconnus.
function Resolve-Modules($catalogue, [string[]]$wanted) {
  $known = @{}; foreach ($m in $catalogue) { $known[$m.id] = $m }
  $set = New-Object System.Collections.Generic.List[string]
  foreach ($m in $catalogue) { if ($m.socle) { $set.Add($m.id) } }
  $queue = New-Object System.Collections.Queue
  foreach ($w in $wanted) { if ($known.ContainsKey($w)) { $queue.Enqueue($w) } }
  while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if ($set.Contains($id)) { continue }
    $set.Add($id)
    foreach ($d in @($known[$id].dependances)) { if ($d -and $known.ContainsKey($d)) { $queue.Enqueue($d) } }
  }
  return @($catalogue | Where-Object { $set.Contains($_.id) } | ForEach-Object { $_.id })
}

function Choose-Modules($catalogue) {
  Step 'Modules'
  $optional = @($catalogue | Where-Object { -not $_.socle })
  Say ('Toujours installés : ' + (($catalogue | Where-Object { $_.socle } | ForEach-Object { $_.nom }) -join ', '))
  if ($Silencieux -or $optional.Count -eq 0) { return Resolve-Modules $catalogue @($optional | ForEach-Object { $_.id }) }
  for ($i = 0; $i -lt $optional.Count; $i++) {
    $m = $optional[$i]
    $tag = if ($m.noyau) { ' [compilé]' } else { '' }
    Say ("{0,3}. {1}{2} — {3}" -f ($i + 1), $m.nom, $tag, $m.description)
  }
  Say 'Tous sont proposés. Tu pourras en ajouter ou en retirer plus tard (Administration › Modules).'
  $a = Read-Host 'Numéros des modules à NE PAS installer (ex. 2,5,9), Entrée pour tout installer'
  $skip = @()
  foreach ($n in ($a -split '[,; ]+' | Where-Object { $_ -match '^\d+$' })) { $skip += $optional[[int]$n - 1].id }
  return Resolve-Modules $catalogue @($optional | Where-Object { $skip -notcontains $_.id } | ForEach-Object { $_.id })
}

function Build-Jarvis($catalogue, [string[]]$chosen) {
  Step 'Compilation (10 à 20 minutes la première fois, quelques minutes ensuite)'
  $features = @($catalogue | Where-Object { $_.noyau -and ($chosen -contains $_.id) } | ForEach-Object { $_.id })
  $env:CARGO_TARGET_DIR = $Target
  Push-Location $Src
  try {
    # Coquille et moteur vocal tels quels ; le noyau (jarvis-app) avec les seules parties noyau des modules choisis.
    & cargo build --release -p jarvis-shell -p jarvis-voice
    if ($LASTEXITCODE -ne 0) { Fail "la compilation a échoué (journal ci-dessus). Les sources restent dans $Src pour diagnostic." }
    $coreArgs = @('build', '--release', '-p', 'jarvis-app', '--no-default-features')
    if ($features.Count) { $coreArgs += @('--features', ($features -join ',')) }
    & cargo @coreArgs
    if ($LASTEXITCODE -ne 0) { Fail "la compilation a échoué (journal ci-dessus). Les sources restent dans $Src pour diagnostic." }
  } finally { Pop-Location }
}

# --- Copie du programme ------------------------------------------------------------------------------------------------

function Copy-Program([string]$tag, [string[]]$chosen, [string]$canal) {
  Step 'Installation du programme'
  $rel = Join-Path $Target 'release'
  if (Test-Path $App) {
    if (Test-Path $Prev) { Remove-Item $Prev -Recurse -Force }
    Move-Item $App $Prev
  }
  New-Item -ItemType Directory -Force $App | Out-Null
  foreach ($f in Get-ChildItem $rel -File | Where-Object { $_.Extension -in '.exe', '.dll' -and $_.Name -ne 'jarvis-launcher.exe' }) {
    Copy-Item $f.FullName $App
  }
  foreach ($d in 'ui', 'assets', 'skills', 'tools', 'browser-extension', 'web-nomade') {
    $s = Join-Path $Src $d
    if (Test-Path $s) { Copy-Item $s (Join-Path $App $d) -Recurse }
  }
  $mods = Join-Path $App 'modules'
  New-Item -ItemType Directory -Force $mods | Out-Null
  Copy-Item (Join-Path $Src 'modules\catalogue.json') $mods
  foreach ($id in $chosen) {
    $s = Join-Path $Src "modules\$id"
    if (Test-Path $s) {
      Copy-Item $s (Join-Path $mods $id) -Recurse
      $core = Join-Path $mods "$id\core"
      if (Test-Path $core) { Remove-Item $core -Recurse -Force }
    }
  }
  Copy-Item (Join-Path $Src 'installer.ps1') $App
  if (Test-Path (Join-Path $Src 'modeles.json')) { Copy-Item (Join-Path $Src 'modeles.json') $App }
  Write-Install ([ordered]@{
    version   = $tag.TrimStart('v')
    tag       = $tag
    canal     = $canal
    depot     = $Depot
    modules   = $chosen
    installeLe = (Get-Date).ToString('o')
  })
  if (Test-Path $Src) { Remove-Item $Src -Recurse -Force }
  Say "Programme installé dans $App ; sources supprimées." 'Green'
}

function Restore-Previous {
  if (Test-Path $Prev) {
    if (Test-Path $App) { Remove-Item $App -Recurse -Force }
    Move-Item $Prev $App
    Say 'Version précédente rétablie.' 'Yellow'
  }
}

# --- Modèles -----------------------------------------------------------------------------------------------------------

function Get-Models([string[]]$chosen) {
  $manifest = Join-Path $App 'modeles.json'
  if (-not (Test-Path $manifest)) { Say 'Pas de liste de modèles dans cette version.' 'Yellow'; return }
  $m = Get-Content $manifest -Raw -Encoding UTF8 | ConvertFrom-Json
  $packs = @($m.paquets | Where-Object { -not $_.module -or ($chosen -contains $_.module) })
  $models = Join-Path $Data 'models'
  $cache = Join-Path $Build 'modeles'
  New-Item -ItemType Directory -Force $models, $cache | Out-Null
  $todo = @($packs | Where-Object { -not (Test-Path (Join-Path $models ".paquet-$($_.sha256)")) })
  if ($todo.Count -eq 0) { Say 'Modèles déjà présents.' 'Green'; return }
  $total = ($todo | Measure-Object octets -Sum).Sum / 1GB
  Step ("Modèles vocaux : {0} paquet(s), {1:N1} Go" -f $todo.Count, $total)
  foreach ($p in $todo) {
    $zip = Join-Path $cache $p.fichier
    $url = "https://github.com/$($m.depot)/releases/download/$($m.version)/$($p.fichier)"
    $ok = (Test-Path $zip) -and ((Get-FileHash $zip -Algorithm SHA256).Hash -eq $p.sha256.ToUpper())
    for ($try = 1; -not $ok -and $try -le 3; $try++) {
      Say ("Téléchargement de {0} ({1:N0} Mo)…" -f $p.fichier, ($p.octets / 1MB))
      & curl.exe -L --fail --retry 3 -C - -o $zip $url
      $ok = (Test-Path $zip) -and ((Get-FileHash $zip -Algorithm SHA256).Hash -eq $p.sha256.ToUpper())
      if (-not $ok -and (Test-Path $zip)) { Remove-Item $zip -Force }
    }
    if (-not $ok) { Fail "téléchargement de $($p.fichier) impossible ou empreinte incorrecte." }
    Expand-Archive -Path $zip -DestinationPath $models -Force
    Set-Content -Path (Join-Path $models ".paquet-$($p.sha256)") -Value $p.fichier
    Remove-Item $zip -Force
  }
  Say 'Modèles installés.' 'Green'
}

# --- Raccourcis --------------------------------------------------------------------------------------------------------

function New-Shortcuts {
  $sh = New-Object -ComObject WScript.Shell
  $targets = @(
    (Join-Path ([Environment]::GetFolderPath('Programs')) 'Jarvis.lnk'),
    (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Jarvis.lnk')
  )
  foreach ($t in $targets) {
    $l = $sh.CreateShortcut($t)
    $l.TargetPath = Join-Path $App 'jarvis.exe'
    $l.WorkingDirectory = $App
    $l.Description = 'Jarvis, assistant vocal'
    $l.Save()
  }
}

# --- Actions -----------------------------------------------------------------------------------------------------------

function Pick-Tag([string]$canal) {
  if ($Version) { return $Version }
  $r = @(Get-Releases $canal)
  if ($r.Count -eq 0) { Fail "aucune version publiée sur github.com/$Depot." }
  return $r[0].Tag
}

# Récupère et compile `tag` avec les modules `wanted` (Jarvis peut rester ouvert) ; note la version prête.
function Prepare([string]$tag, [string[]]$wanted, [bool]$ask) {
  if (Test-Path $Ready) { Remove-Item $Ready -Force }
  Ensure-Prerequisites
  Fetch-Sources $tag
  $catalogue = Read-Catalogue
  $chosen = if ($ask) { Choose-Modules $catalogue } else { Resolve-Modules $catalogue $wanted }
  Build-Jarvis $catalogue $chosen
  $json = [ordered]@{ tag = $tag; modules = $chosen; canal = $Canal; le = (Get-Date).ToString('o') } | ConvertTo-Json
  [IO.File]::WriteAllText($Ready, $json, (New-Object Text.UTF8Encoding $false))
  Say "Version $tag compilée, prête à être installée." 'Green'
  return $chosen
}

# Installe la version préparée (Jarvis fermé), puis les modèles qui manquent.
function Apply([bool]$restart) {
  if (-not (Test-Path $Ready) -or -not (Test-Path $Src)) { Fail 'aucune version préparée (lance -Preparer d''abord).' }
  $r = Get-Content $Ready -Raw -Encoding UTF8 | ConvertFrom-Json
  Wait-JarvisClosed
  try { Copy-Program $r.tag @($r.modules) $r.canal } catch { Restore-Previous; Fail $_.Exception.Message }
  Remove-Item $Ready -Force
  if (-not $SansModeles) { Get-Models @($r.modules) }
  if ($restart) { Start-Jarvis }
}

New-Item -ItemType Directory -Force $Base | Out-Null
$info = Read-Install
if ($info -and $info.depot) { $Depot = $info.depot }
if (-not $Canal) { $Canal = if ($info -and $info.canal) { $info.canal } else { 'stable' } }

if ($LibererPlace) {
  if (Test-Path $Target) { Remove-Item $Target -Recurse -Force }
  Say 'Cache de compilation vidé (la prochaine mise à jour recompilera tout).' 'Green'
  exit 0
}

if ($Desinstaller) {
  Step 'Désinstallation de Jarvis'
  Wait-JarvisClosed 30
  foreach ($p in $App, $Prev, $Build) { if (Test-Path $p) { Remove-Item $p -Recurse -Force } }
  foreach ($l in (Join-Path ([Environment]::GetFolderPath('Programs')) 'Jarvis.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Jarvis.lnk')) {
    if (Test-Path $l) { Remove-Item $l -Force }
  }
  if ((Test-Path $Data) -and (Ask "Supprimer aussi TOUTES tes données ($Data) ? Pense à les exporter avant (Administration › Mes données)." $false)) {
    Remove-Item $Data -Recurse -Force
    Say 'Données supprimées.' 'Yellow'
  } else {
    Say "Données gardées dans $Data (une réinstallation les retrouvera)." 'Green'
  }
  exit 0
}

if ($Restaurer) {
  $core = Join-Path $App 'jarvis-core.exe'
  if (-not (Test-Path $core)) { Fail 'Jarvis n''est pas installé.' }
  Wait-JarvisClosed
  $a = @('donnees', 'importer', $Restaurer, '--oui')
  if ($MotDePasse) { $a += @('--mot-de-passe', $MotDePasse) }
  & $core @a
  if ($LASTEXITCODE -ne 0) { Fail 'restauration impossible (voir ci-dessus).' }
  if (-not $Silencieux -and (Ask 'Lancer Jarvis ?')) { Start-Jarvis }
  exit 0
}

if ($Appliquer) { Apply (-not $SansRelancer); exit 0 }

if ($Preparer) {
  if (-not $info) { Fail 'Jarvis n''est pas installé.' }
  $null = Prepare (Pick-Tag $Canal) @($info.modules) $false
  exit 0
}

if ($AjouterModule -or $RetirerModule) {
  if (-not $info) { Fail 'Jarvis n''est pas installé.' }
  $current = @($info.modules)
  $cat = (Get-Content (Join-Path $App 'modules\catalogue.json') -Raw -Encoding UTF8 | ConvertFrom-Json).modules
  if ($RetirerModule) {
    $m = $cat | Where-Object { $_.id -eq $RetirerModule }
    if (-not $m) { Fail "module « $RetirerModule » inconnu." }
    if ($m.socle) { Fail "« $($m.nom) » fait partie du socle : il ne se retire pas." }
    $needs = @($cat | Where-Object { $current -contains $_.id -and @($_.dependances) -contains $RetirerModule } | ForEach-Object { $_.nom })
    if ($needs.Count) { Fail ("d'autres modules en ont besoin : " + ($needs -join ', ')) }
    $info.modules = @($current | Where-Object { $_ -ne $RetirerModule })
    if (-not $m.noyau) {
      # Interface seule : retrait immédiat ; ses données restent dans donnees\.
      $d = Join-Path $App "modules\$RetirerModule"
      if (Test-Path $d) { Remove-Item $d -Recurse -Force }
      Write-Install $info
      Say 'Module retiré ; ses données sont gardées.' 'Green'
      exit 0
    }
    $null = Prepare $info.tag @($info.modules) $false
    Say 'Le module sera retiré au prochain redémarrage de Jarvis.' 'Green'
    exit 0
  }
  $m = $cat | Where-Object { $_.id -eq $AjouterModule }
  if (-not $m) { Fail "module « $AjouterModule » inconnu dans $($info.tag)." }
  $wanted = @($current + $AjouterModule)
  $needCore = @($cat | Where-Object { $_.noyau -and (Resolve-Modules $cat $wanted) -contains $_.id -and $current -notcontains $_.id }).Count -gt 0
  if (-not $needCore) {
    # Interface seule (et dépendances sans partie noyau) : copiés tels quels, sans recompiler.
    Fetch-Sources $info.tag
    $chosen = Resolve-Modules (Read-Catalogue) $wanted
    foreach ($id in $chosen | Where-Object { $current -notcontains $_ }) {
      $s = Join-Path $Src "modules\$id"; $d = Join-Path $App "modules\$id"
      if (Test-Path $d) { Remove-Item $d -Recurse -Force }
      Copy-Item $s $d -Recurse
      if (Test-Path (Join-Path $d 'core')) { Remove-Item (Join-Path $d 'core') -Recurse -Force }
    }
    $info.modules = $chosen
    Write-Install $info
    Remove-Item $Src -Recurse -Force
    Get-Models $chosen
    Say 'Module ajouté : il apparaît sans relancer Jarvis.' 'Green'
    exit 0
  }
  $null = Prepare $info.tag $wanted $false
  Say 'Module compilé : il sera installé au prochain redémarrage de Jarvis.' 'Green'
  exit 0
}

if ($MiseAJour) {
  if (-not $info) { Fail 'Jarvis n''est pas installé : lance le script sans -MiseAJour.' }
  $tag = Pick-Tag $Canal
  Say "Mise à jour de Jarvis $($info.tag) vers $tag"
  $null = Prepare $tag @($info.modules) $false
  if ((Running).Count -gt 0) { Say 'Ferme Jarvis (icône de la zone de notification › Quitter) pour terminer la mise à jour.' 'Yellow' }
  Apply $true
  exit 0
}

# Première installation (ou réinstallation).
Say 'Jarvis — installation' 'Cyan'
$tag = Pick-Tag $Canal
Say "Version : $tag (canal $Canal), depuis github.com/$Depot"
$chosen = Prepare $tag $Modules (-not $Modules)
Apply $false
New-Shortcuts
if ((-not (Test-Path (Join-Path $Data 'VERSION.json'))) -and -not $Silencieux) {
  if (Ask 'Restaurer tes données depuis une archive .jarvis (autre PC, ancienne installation) ?' $false) {
    $file = Read-Host 'Chemin du fichier .jarvis'
    $pw = Read-Host 'Mot de passe de l''archive (Entrée si aucun)'
    $a = @('donnees', 'importer', $file.Trim('"'), '--oui')
    if ($pw) { $a += @('--mot-de-passe', $pw) }
    & (Join-Path $App 'jarvis-core.exe') @a
  }
}
Say ''
Say "Jarvis $tag est installé. Raccourcis : menu Démarrer et Bureau." 'Green'
if (Ask 'Lancer Jarvis maintenant ?') { Start-Jarvis }