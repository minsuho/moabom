# 파일을 VirusTotal에 올리고 검사가 끝날 때까지 기다린 뒤, 릴리스 노트에 넣을 마크다운 줄을 낸다.
# 환경변수 VT_API_KEY가 필요하다. 무료 API는 분당 4회까지라 요청 사이를 띄운다.
param(
  [Parameter(Mandatory)] [string[]] $Files,
  [Parameter(Mandatory)] [string[]] $Labels
)
$ErrorActionPreference = 'Stop'
$headers = @{ 'x-apikey' = $env:VT_API_KEY }
$api = 'https://www.virustotal.com/api/v3'

function Call([string] $method, [string] $uri, $form) {
  Start-Sleep -Seconds 16
  if ($form) { Invoke-RestMethod -Method $method -Uri $uri -Headers $headers -Form $form }
  else { Invoke-RestMethod -Method $method -Uri $uri -Headers $headers }
}

for ($i = 0; $i -lt $Files.Count; $i++) {
  $f = Get-Item $Files[$i]
  $hash = (Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLower()
  $link = "https://www.virustotal.com/gui/file/$hash"
  $stats = $null
  try {
    # 32MB가 넘는 파일은 따로 받은 업로드 주소로 올려야 한다.
    $uri = "$api/files"
    if ($f.Length -gt 32MB) { $uri = (Call Get "$api/files/upload_url").data }
    $id = (Call Post $uri @{ file = $f }).data.id
    $deadline = (Get-Date).AddMinutes(15)
    while ((Get-Date) -lt $deadline) {
      $a = (Call Get "$api/analyses/$id").data.attributes
      if ($a.status -eq 'completed') { $stats = $a.stats; break }
    }
  } catch {
    Write-Warning "VirusTotal 검사 실패 ($($f.Name)): $_"
  }
  if ($stats) {
    $bad = $stats.malicious + $stats.suspicious
    $total = $bad + $stats.undetected + $stats.harmless
    "- [$($Labels[$i])]($link): 백신 $total개 중 $bad개 탐지"
  } else {
    "- [$($Labels[$i])]($link): 검사 중이에요. 링크를 열면 결과가 보여요."
  }
}
