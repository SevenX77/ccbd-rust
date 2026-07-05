# Req1 Phase 2 — Real-Machine Validation Runbook

**Who runs this:** you (the user), on your own Windows 11 + WSL2 machine.
**When:** any time now. v1.3.0 is already public on `SevenX77/ah` (shipped ahead of this sign-off per the "ship all completed features" directive); this runbook is the post-release confirmation of the Windows provisioning path.
**Why:** the WSL2-feature-enable + reboot + distro-OOBE + in-WSL install path *cannot* be tested on our Linux CI. Your PASS here closes the Studio Req1 real-machine gate; any FAIL is a real bug we fix in a v1.3.x follow-up (not something you work around).

Each step lists: **do**, **run**, **expect**, and a **[ ] sign-off** box. If any step's actual result differs from **expect**, stop and paste me the output — that's a real-machine bug we fix before release, not something you work around.

---

## Two things about how the helper behaves (read first)

1. **Mutation happens only under `-Fix`.** Without `-Fix`, the helper only *reports* / advances read-only and stops before installing anything. Every step that actually changes your machine passes **`-Fix`**. `-Yes` additionally lets it run `wsl.exe --shutdown` itself instead of just telling you to.
2. **The helper drives forward and stops at three human boundaries**, then you resume past each:
   - **Boundary A — reboot** (after enabling WSL features) → exit `11`
   - **Boundary B — distro first-launch / OOBE** (after installing Ubuntu) → exit `12`
   - **Boundary C — WSL shutdown** (after installing `ah` + distro-local setup) → exit `10` (with `-Yes` it may run the shutdown itself and continue straight to `pass`)
   - then **terminal `pass`** → exit `0`

   So the run is: fix → (reboot) → resume → (OOBE) → resume → (shutdown) → resume → pass. Between boundaries the helper does several stages in one command — don't expect one command per stage.

### `ah` install URL (already filled in)
Steps that install `ah` **inside WSL** use the **public release** `v1.3.0` (the current `latest` on `SevenX77/ah`). Every command below that reaches the install stage already carries `-AhInstallUrl "https://github.com/SevenX77/ah/releases/download/v1.3.0/ah-installer.sh"` and `-ExpectedAhVersion "1.3.0"`. Note: v1.3.0 was tagged/published ahead of this sign-off (per the operator's "ship all completed features" directive). Running this runbook now is the **post-release confirmation** of the Windows provisioning path that Linux CI can't cover — a PASS closes the Studio Req1 real-machine gate; any FAIL is a real bug we fix in a v1.3.x follow-up.

---

## Step 0 — Prep the machine and get the assets

**Do:** Use a Windows 11 machine (or clean VM) where WSL2 is **not yet enabled** — that's the true first-run path. Open a **normal (non-admin) PowerShell** window. (The helper self-elevates only for the DISM feature-enable step, via a UAC prompt.)

**Run:** download the three provisioning assets from the candidate pre-release into one folder and cd there:
```powershell
$dir = "$HOME\Downloads\ah-provision"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
cd $dir
$base = "https://github.com/SevenX77/ah/releases/download/v1.3.0"
foreach ($f in "provision-ah-wsl.ps1","AhProvisioning.psm1","enable-ah-wsl-features.ps1") {
  Invoke-WebRequest -Uri "$base/$f" -OutFile $f
}
Get-ChildItem   # confirm all three present in the SAME folder
```

**Expect:** all three files present in one folder. (`provision-ah-wsl.ps1` auto-loads `AhProvisioning.psm1` from the same directory; `enable-ah-wsl-features.ps1` is the elevated DISM child it invokes.)

**[ ] sign-off:**

---

## Step 1 — Read-only check from a clean standard-user shell (spec gate §P2-6 "helper starts")

**Do:** confirm the downloaded script launches the helper as a standard user (no admin, no state yet).

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Check -Json
echo "EXIT=$LASTEXITCODE"
```

**Expect:** prints a JSON envelope with `overall_status` and a `steps` array; does **not** crash and does **not** need admin just to *read* status. On a WSL-disabled machine `overall_status` is **`fail`** with the feature steps marked as fixable / needing admin (that's the correct "not yet provisioned" report, not an error in the script).

**[ ] sign-off:**

---

## Step 2 — Enable WSL2 features → reboot boundary (spec gate §P2-2, writes `%LOCALAPPDATA%\ah\setup-state.json`)

**Do:** run the actual fix. A **UAC prompt** appears (the elevated child runs DISM to enable `Microsoft-Windows-Subsystem-Linux` + `VirtualMachinePlatform`). Approve it.

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Fix -Yes
echo "EXIT=$LASTEXITCODE"
Get-Content "$env:LOCALAPPDATA\ah\setup-state.json"
```

**Expect:**
- UAC prompt → after approval, DISM enables the two features.
- Status ends at **`needs_windows_reboot`**, prints resume instructions, **`EXIT=11`**.
- `setup-state.json` exists and shows a pending-reboot state.

**[ ] sign-off:**

---

## Step 3 — Idempotent before reboot: rerun does NOT repeat DISM or install (spec gate §P2-2 rerun)

**Do:** without rebooting, resume once more.

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Resume -Fix -Yes
echo "EXIT=$LASTEXITCODE"
```

**Expect:** it **reprints the pending-reboot boundary** (`needs_windows_reboot`, **`EXIT=11`**); it must **not** re-run DISM or attempt distro install. (Bonus check — a *bare* rerun with no flags now nudges you to resume rather than starting over:)
```powershell
.\provision-ah-wsl.ps1
```
**Expect (bare):** it detects the pending state and tells you to resume — it does **not** start a fresh plan.

**[ ] sign-off:**

---

## Step 4 — Reboot

**Do:** reboot Windows normally. After logging back in, reopen non-admin PowerShell:
```powershell
cd $HOME\Downloads\ah-provision
```

**Expect:** clean reboot, back at a normal shell.

**[ ] sign-off:**

---

## Step 5 — Resume after reboot → distro install → OOBE boundary (spec gates §P2-2 post-reboot, §P2-3)

**Do:** resume. The helper re-probes features (no repeat DISM), confirms WSL2 is live, then installs the Ubuntu distro and stops at the first-launch (OOBE) boundary.

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Resume -Fix -Yes -Distro Ubuntu -AhInstallUrl "https://github.com/SevenX77/ah/releases/download/v1.3.0/ah-installer.sh" -ExpectedAhVersion "1.3.0"
echo "EXIT=$LASTEXITCODE"
wsl.exe --status
```

**Expect:** no repeat DISM; `wsl.exe --status` responds (WSL2 active); a fresh Ubuntu install runs; helper stops at **`needs_distro_first_launch`**, **`EXIT=12`**, with instructions to complete the OOBE.

> **If you already had a WSL1 Ubuntu** (spec gate §P2-2 WSL1): instead of a clean install, the helper either converts it to WSL2 or stops with exact backup/conversion guidance, and runs **no** in-distro setup while the distro version is still `1`. Record which branch happened.

**[ ] sign-off:**

---

## Step 6 — Complete OOBE (create your Ubuntu user)

**Do:** launch Ubuntu once (Start menu → Ubuntu, or `wsl -d Ubuntu`), complete **username + password** creation, then exit back to PowerShell.

**Expect:** you land at a normal Ubuntu user (non-root) prompt, then close it.

**[ ] sign-off:**

---

## Step 7 — Resume → install `ah` in WSL → distro-local setup → shutdown boundary (spec gates §P2-3 / §P2-4 / §P2-5)

**Do:** resume **without `-Yes`** so the helper stops at the WSL-shutdown boundary and you exercise it explicitly (the spec gate wants: reach `needs_wsl_shutdown` → shut down → resume → pass). In this one command the helper detects your non-root user, installs `ah` into `$HOME/.local/bin`, verifies the version by absolute path, runs the distro-local `ah setup --fix`, and then stops at the shutdown boundary.

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Resume -Fix -Distro Ubuntu -AhInstallUrl "https://github.com/SevenX77/ah/releases/download/v1.3.0/ah-installer.sh" -ExpectedAhVersion "1.3.0"
echo "EXIT=$LASTEXITCODE"
```

**Expect:** **`EXIT=10`** (`needs_wsl_shutdown`) — it installed `ah` and ran distro-local setup, and now instructs you to shut down WSL.

> Note: if you instead pass `-Yes` here, the helper runs `wsl.exe --shutdown` itself and continues straight to `pass` (`EXIT=0`) — no `10`. We drop `-Yes` on this command specifically to prove the boundary + resume path.

**[ ] sign-off (boundary reached):**

---

## Step 7b — Shut down and resume to pass (spec gate §P2-5)

**Run:**
```powershell
wsl.exe --shutdown
.\provision-ah-wsl.ps1 -Resume -Fix -Yes -Distro Ubuntu -AhInstallUrl "https://github.com/SevenX77/ah/releases/download/v1.3.0/ah-installer.sh" -ExpectedAhVersion "1.3.0"
echo "EXIT=$LASTEXITCODE"
```

**Expect:** now **`EXIT=0`** (`pass`).

Confirm the install landed:
```powershell
wsl.exe -d Ubuntu -- sh -lc '"$HOME/.local/bin/ah" --version'
```
**Expect:** prints `1.3.0`.

**[ ] sign-off:**

---

## Step 8 — Idempotent install: re-run skips reinstall (spec gate §P2-4 rerun)

**Run:**
```powershell
.\provision-ah-wsl.ps1 -Resume -Fix -Yes -Distro Ubuntu -AhInstallUrl "https://github.com/SevenX77/ah/releases/download/v1.3.0/ah-installer.sh" -ExpectedAhVersion "1.3.0"
```

**Expect:** because `ah` is already at `1.3.0`, it **skips** the download/install (no redundant reinstall) and returns `pass` / already-provisioned. *(This is exactly the idempotency bug we fixed in PR #96 — this step proves it on a real machine.)*

**[ ] sign-off:**

---

## Step 9 — Final end-to-end proof (spec gate §P2-5 final)

**Run:** the authoritative check the spec requires — Linux `ah` self-check inside WSL:
```powershell
wsl.exe -d Ubuntu -- sh -lc '"$HOME/.local/bin/ah" setup --check --json'
echo "EXIT=$LASTEXITCODE"
```

**Expect:** **`EXIT=0`** and a JSON report showing the in-WSL `ah` environment is healthy (systemd + tmux present).

**[ ] sign-off:**

---

## Step 10 — Record the verdict

- **All PASS** → reply to me: **"Req1 Phase 2 real-machine validation PASS"**. I then cut the single release tag (Req2 + Req3 + tmux follow-terminal + full Req1 Phase 0/1/2) and publish to `SevenX77/ah`, promoting the candidate build to the public release.
- **Any FAIL / unexpected output** → paste me the failing step's full output. We fix it (mock-CI + re-run the affected step) before any tag.

**[ ] final verdict:** PASS / FAIL

---

### Exit-code quick reference (`provision-ah-wsl.ps1`)
| code | meaning |
|---|---|
| 0 | `pass` / `fixed` |
| 10 | `needs_wsl_shutdown` |
| 11 | `needs_windows_reboot` |
| 12 | `needs_distro_first_launch` |
| 13 | `permission_denied` (UAC declined / not elevated) |
| 2 | `unsupported` (e.g. WSL1 conversion blocked) |
| 1 | other error |

### Key flags
- `-Check` read-only report · `-Fix` perform changes · `-Resume` continue from saved state · `-Yes` auto-run `wsl --shutdown` · `-Distro <name>` (default `Ubuntu`) · `-AhInstallUrl <url>` candidate build · `-ExpectedAhVersion 1.3.0` · `-Json` machine-readable envelope · `-DryRun` no host mutations · `-StatePath <path>` override state file.
