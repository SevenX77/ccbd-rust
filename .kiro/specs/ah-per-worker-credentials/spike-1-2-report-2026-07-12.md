# Per-Worker Credentials · Spike#1/#2 Report · 2026-07-12

## Safety Envelope

- All Claude runs used fresh `/tmp/cred-spike-*` directories with isolated `HOME`, `CLAUDE_CONFIG_DIR`, and `ANTHROPIC_CONFIG_DIR`.
- I did not read or write any real `~/.claude/.credentials.json` or worker shared credential file. One initial `ls -la ~/.claude` showed the current ah sandbox HOME contains a `.credentials.json` symlink to a shared path; after that, all experiment runs overrode HOME/CONFIG paths.
- `claude --help` did not document a separate config-dir flag, but the installed binary contains `CLAUDE_CONFIG_DIR` and `ANTHROPIC_CONFIG_DIR`. An isolated `claude auth status` probe confirmed `$CLAUDE_CONFIG_DIR/.credentials.json` and `$ANTHROPIC_CONFIG_DIR/configs/<profile>.json` are used.
- No real tokens appear in this report. Dummy values are redacted in log excerpts.

## Spike#2: Dummy/Invalid Refresh Token Behavior

### Method

1. Created isolated profile directories under `/tmp`.
2. Probed credential shapes with `timeout`-wrapped `claude auth status --json`.
3. The shape accepted as logged in was:
   - `$ANTHROPIC_CONFIG_DIR/configs/spike.json`
   - `ANTHROPIC_PROFILE=spike`
   - `authentication.type = "user_oauth"`
   - `authentication.credentials_path = <isolated credentials file>`
4. Ran:

```bash
timeout 30 env -i \
  HOME=/tmp/cred-spike-s2-snake-jdmiOj/home \
  CLAUDE_CONFIG_DIR=/tmp/cred-spike-s2-snake-jdmiOj/config \
  ANTHROPIC_CONFIG_DIR=/tmp/cred-spike-s2-snake-jdmiOj/config \
  ANTHROPIC_PROFILE=spike \
  strace -f -e trace=network -o network.strace \
  claude --safe-mode --debug api --debug-file debug.log \
    -p "Reply with one word: ok" --output-format json \
    --no-session-persistence --permission-mode dontAsk --tools ""
```

Credentials used:

```json
{"access_token":"[DUMMY]","refresh_token":"[DUMMY]","expires_at":1}
```

I also tried adding a UUID-shaped `client_id`; Claude still reported `client_id empty`, so the fabricated file was not considered refreshable.

### Observations

- Exit code: `124` from `timeout 30`.
- stdout: empty.
- stderr: empty.
- No crash was observed.
- No recursive deletion or destructive credential-directory mutation was observed. The isolated credential file remained present and unchanged.
- Claude entered a retry loop against a local auth failure; attempts reached at least `attempt 6/11` before the timeout killed the process.
- The exact dummy-refresh POST to `/v1/oauth/token` was not reached with fabricated isolated credentials.
- `strace` showed network activity to `api.anthropic.com:443`, but not a distinguishable OAuth token POST. The debug log explains why: the local profile credential was considered expired and non-refreshable.

Key sanitized log:

```text
RC=124
stdout: <empty>
stderr: <empty>

[INFO] Using Anthropic profile auth (profile-explicit); this takes precedence over any stored claude.ai login
[ERROR] WIF auth header resolution failed: Access token at /tmp/cred-spike-s2-snake-jdmiOj/config/credentials/spike.json has expired and no refresh is available (client_id empty, refresh_token=[REDACTED] set)
[ERROR] API error (attempt 1/11): Access token at /tmp/cred-spike-s2-snake-jdmiOj/config/credentials/spike.json has expired and no refresh is available (client_id empty, refresh_token=[REDACTED] set)
...
[ERROR] API error (attempt 6/11): Access token at /tmp/cred-spike-s2-snake-jdmiOj/config/credentials/spike.json has expired and no refresh is available (client_id empty, refresh_token=[REDACTED] set)
```

Network excerpt:

```text
sendmmsg(... "api\tanthropic\3com" ...)
connect(... sin_port=htons(443), sin_addr=inet_addr("160.79.104.10")) = -1 EINPROGRESS
sendto(... TLS ClientHello ...)
recvfrom(... TLS response ...)
```

Files after run:

```text
f /tmp/cred-spike-s2-snake-jdmiOj/config/configs/spike.json 163
f /tmp/cred-spike-s2-snake-jdmiOj/config/credentials/spike.json 99
f /tmp/cred-spike-s2-snake-jdmiOj/debug.log 16702
f /tmp/cred-spike-s2-snake-jdmiOj/network.strace 47806
f /tmp/cred-spike-s2-snake-jdmiOj/stdout.log 0
f /tmp/cred-spike-s2-snake-jdmiOj/stderr.log 0
```

### Conclusion

**Blocked / not fully verified.** In a safe isolated environment with only fabricated credentials, Claude does not reach the invalid-refresh-token server path. It treats the profile credential as expired and non-refreshable, then retries the local auth error until `timeout 30` kills it.

Partial signal:

- Fails the desired pass criterion of a clean, prompt "needs login" exit: it produced no stdout/stderr and timed out.
- No crash or destructive deletion was observed.
- The exact behavior for a syntactically real but semantically invalid refresh token remains unverified because I do not have an isolated, non-live refreshable credential.

## Spike#1: HTTPS_PROXY / SSL Pinning for `/v1/oauth/token`

### Method

1. Checked for `mitmproxy`/`mitmdump`; not installed initially.
2. Ran a `timeout 120` wrapped install:

```bash
timeout 120 bash -lc 'pip install --user mitmproxy ...'
```

`pip` installed mitmproxy under the current ah sandbox user path:

```text
Successfully installed ... mitmproxy-12.2.3 ...
WARNING: The scripts mitmdump, mitmproxy and mitmweb are installed in '/home/sevenx/.cache/ah/sandboxes/45530fd0c4c3/.local/bin' which is not on PATH.
```

The install command returned nonzero only because the script directory was not on `PATH`; invoking `~/.local/bin/mitmdump` directly worked.

3. Started isolated `mitmdump` with its own confdir and CA:

```bash
mitmdump --set confdir=/tmp/cred-spike-s1-proxy2-BfQypR/mitm \
  --listen-host 127.0.0.1 --listen-port 58484 -s addon.py
```

4. Ran Claude with:

```bash
HTTPS_PROXY=http://127.0.0.1:58484
HTTP_PROXY=http://127.0.0.1:58484
NODE_EXTRA_CA_CERTS=/tmp/cred-spike-s1-proxy2-BfQypR/mitm/mitmproxy-ca-cert.pem
```

Same isolated expired dummy profile credentials were used.

### Observations

- Exit code: `124` from `timeout 20`, due to the same local non-refreshable auth retry loop as Spike#2.
- The debug log confirms Claude loaded the mitm CA from `NODE_EXTRA_CA_CERTS`.
- `mitmdump` successfully decrypted ordinary HTTPS traffic to `api.anthropic.com`; no SSL pinning blocked those requests.
- No `/v1/oauth/token` request was observed, because the fabricated credential never reached refresh.

Sanitized mitm log:

```text
HTTP(S) proxy listening at 127.0.0.1:58484.
client connect
server connect api.anthropic.com:443 (160.79.104.10:443)
GET api.anthropic.com /mcp-registry/v0/servers?version=latest&limit=100&visibility=commercial%2Cgsuite%2Centerprise%2Chealth
<< 200 OK 79.1k
...
POST api.anthropic.com /api/event_logging/v2/batch
<< 200 OK 57b
```

Debug CA/proxy evidence:

```text
CA certs: stores=bundled,system, extraCertsPath=/tmp/cred-spike-s1-proxy2-BfQypR/mitm/mitmproxy-ca-cert.pem
CA certs: Appended extra certificates from NODE_EXTRA_CA_CERTS (/tmp/cred-spike-s1-proxy2-BfQypR/mitm/mitmproxy-ca-cert.pem)
mTLS: Creating HTTPS agent with custom certificates
```

Auth failure preventing OAuth-token test:

```text
WIF auth header resolution failed: Access token at /tmp/cred-spike-s1-proxy2-BfQypR/config/credentials/spike.json has expired and no refresh is available (client_id empty, refresh_token=[REDACTED] set)
API error (attempt 1/11): Access token at /tmp/cred-spike-s1-proxy2-BfQypR/config/credentials/spike.json has expired and no refresh is available ...
```

### Conclusion

**Environment partially verified, OAuth-token endpoint not verified.**

- `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` are honored for at least ordinary `api.anthropic.com` HTTPS traffic, and mitmproxy can decrypt it.
- This does **not** prove that `platform.claude.com` or `/v1/oauth/token` lacks SSL pinning, because no isolated refreshable credential was available to trigger that endpoint safely.
- Per the safety rule, I did not use live/shared credentials to force the refresh path.

## Overall Result

- Spike#2 keystone remains **blocked / failed-to-verify exactly** without an isolated refreshable credential. The fabricated isolated credential path shows a problematic retry-until-timeout local failure, not a clean login-required exit.
- Spike#1 remains **not verified for `/v1/oauth/token`**. Proxy interception works for non-token Anthropic HTTPS calls, but the OAuth-token request could not be safely generated.
- Recommended next step: provide a disposable, isolated Claude OAuth credential whose refresh token is safe to invalidate, or authorize a controlled test account login inside a fresh `/tmp/cred-spike-*` HOME/CONFIG directory.

## Spike#2 修正重跑(消费级 claude.ai 登录路径)

### Correction

The earlier run accidentally selected the enterprise/profile auth path by setting `ANTHROPIC_PROFILE` plus config-dir variables. This rerun intentionally did **not** set:

- `ANTHROPIC_PROFILE`
- `CLAUDE_CONFIG_DIR`
- `ANTHROPIC_CONFIG_DIR`

Only `HOME` was isolated. The dummy credential was written to:

```text
/tmp/cred-spike-s2-consumer-7HY3NF/home/.claude/.credentials.json
```

Credential shape:

```json
{
  "claudeAiOauth": {
    "accessToken": "[DUMMY]",
    "refreshToken": "[DUMMY]",
    "expiresAt": 1783799914458,
    "refreshTokenExpiresAt": 1783799914458,
    "scopes": ["user:inference", "user:profile"],
    "subscriptionType": "pro"
  }
}
```

### Command

```bash
timeout 30 env -i \
  PATH="$PATH" \
  HOME=/tmp/cred-spike-s2-consumer-7HY3NF/home \
  USER=spike LOGNAME=spike SHELL=/bin/bash \
  DISABLE_ERROR_REPORTING=1 CLAUDE_CODE_ENABLE_TELEMETRY=0 \
  strace -f -s 200 \
    -e trace=network,openat,statx,readlink,rename,unlink,mkdir,rmdir \
    -o trace.strace \
  claude --safe-mode --debug api --debug-file debug.log \
    -p "Reply with one word: ok" --output-format json \
    --no-session-persistence --permission-mode dontAsk --tools ""
```

I then repeated the same HOME-only credential setup with `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` pointing at an isolated mitmproxy instance, solely to make the HTTPS request path visible. No profile/config-dir variables were set in that run either.

### Observations

- Exit code: `1`.
- stdout was a JSON result, not a crash:

```json
{
  "is_error": true,
  "result": "Failed to authenticate: OAuth session expired and could not be refreshed",
  "terminal_reason": "api_error"
}
```

- stderr: empty.
- No infinite retry loop: the direct strace run completed in about 2 seconds.
- No process crash observed.
- The isolated credential file was rewritten into a local logout/residual state:

```json
{
  "claudeAiOauth": {
    "accessToken": "",
    "refreshToken": "",
    "expiresAt": 0,
    "refreshTokenExpiresAt": 1783799914458,
    "scopes": ["user:inference", "user:profile"],
    "subscriptionType": "pro"
  }
}
```

This destructive write was confined to the fresh `/tmp/cred-spike-*` HOME and did not touch live credentials.

### Evidence: Correct Code Path

The corrected run no longer printed the earlier profile-auth line:

```text
Using Anthropic profile auth (profile-explicit)
```

That string was absent from the corrected debug excerpts. Instead, the debug log showed the consumer OAuth refresh path:

```text
[ERROR] OAuth refresh failed (expected): Request failed with status code 400
[ERROR] API error (attempt 1/11): OAuth refresh token is no longer valid; run /login to re-authenticate
[ERROR] API auth_error: OAuth refresh token is no longer valid; run /login to re-authenticate
```

The syscall trace confirms Claude read the intended isolated consumer credential file and checked for profile config only under the isolated HOME:

```text
openat(... "/tmp/cred-spike-s2-consumer-7HY3NF/home/.claude/.credentials.json", O_RDONLY...) = 13
openat(... "/tmp/cred-spike-s2-consumer-7HY3NF/home/.config/anthropic/active_config", O_RDONLY...) = -1 ENOENT
openat(... "/tmp/cred-spike-s2-consumer-7HY3NF/home/.config/anthropic/configs/default.json", O_RDONLY...) = -1 ENOENT
```

The same trace shows DNS and TLS connection setup for the consumer OAuth host:

```text
sendto(... "\10platform\6claude\3com" ...)
recvfrom(... "\10platform\6claude\3com" ...)
connect(... sin_port=htons(443), sin_addr=inet_addr("160.79.104.10")) = -1 EINPROGRESS
sendto(... TLS ClientHello ... "platform.claude.com" ...)
```

### Evidence: `/v1/oauth/token` POST

The mitmproxy rerun made the encrypted request path explicit:

```text
HTTP(S) proxy listening at 127.0.0.1:58485.
server connect platform.claude.com:443 (160.79.104.10:443)
POST platform.claude.com /v1/oauth/token
RESPONSE 400 platform.claude.com /v1/oauth/token
127.0.0.1:32792: POST https://platform.claude.com/v1/oauth/token
              << 400 Bad Request 99b
```

After that failed refresh, Claude still attempted API calls with an invalid/no-longer-usable bearer token and received 401s:

```text
GET api.anthropic.com /api/oauth/profile
RESPONSE 401 api.anthropic.com /api/oauth/profile
POST api.anthropic.com /v1/messages?beta=true
RESPONSE 401 api.anthropic.com /v1/messages?beta=true
```

Mitm stdout for that run ended with:

```json
{
  "is_error": true,
  "api_error_status": 401,
  "result": "Failed to authenticate. API Error: 401 Invalid bearer token",
  "terminal_reason": "api_error"
}
```

### Conclusion

**Spike#2 corrected rerun: PASS for observability and safety characterization, FAIL for Layer-1 clean-failure hopes.**

What is now verified:

- The HOME-only setup does hit the consumer `claudeAiOauth` plaintext credential path.
- Expired dummy `accessToken` + dummy `refreshToken` triggers a real `POST platform.claude.com /v1/oauth/token`.
- The invalid refresh returns 400.
- Claude does not crash and does not infinite-loop.
- Claude returns a recognizable authentication failure.
- Claude rewrites the isolated credential file into a residual logout state with empty tokens and `expiresAt: 0`.

Design implication:

- A worker with a local, non-symlink, dummy consumer credential will fail locally and observably; it will also mutate its own isolated credential file into a logout residual.
- This is safe only if worker credentials are physically isolated true files. If that file is a symlink to a live/shared credential, this exact behavior is the write-through hazard Module D is trying to eliminate.
