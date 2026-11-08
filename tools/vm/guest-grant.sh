# Runs inside the VM. The image has SIP off, so permissions go straight into TCC's
# database, with no prompt and nobody clicking in the VM. The driver needs no row: it
# runs under `tart exec`, so it holds the image's grants for the guest agent.
db="/Library/Application Support/com.apple.TCC/TCC.db"
grant() { # service bundle_id
    sudo sqlite3 "$db" "INSERT OR REPLACE INTO access (service,client,client_type,auth_value,auth_reason,auth_version,flags) VALUES ('$1','$2',0,2,4,1,0);"
}
for service in kTCCServiceScreenCapture kTCCServiceAccessibility kTCCServicePostEvent; do
    grant "$service" dev.attn.lens
done
# macOS 26 also asks before a stream "bypasses the window picker". An entry here with a
# far hint date stops that dialog, which would otherwise cover the app under test. An app
# is keyed by its file URL, a bare binary by its real path. The guest agent needs one
# because `screencapture` records under it, and its dialog sits over the area's corner.
/usr/bin/python3 -I - "$HOME/Applications/Attn Lens.app" /opt/homebrew/bin/tart-guest-agent <<'PY'
import datetime, os, pathlib, plistlib, sys, urllib.parse
path = pathlib.Path.home() / "Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist"
approvals = plistlib.loads(path.read_bytes()) if path.exists() else {}
now = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
for client in sys.argv[1:]:
    key = ("file://" + urllib.parse.quote(client) + "/" if client.endswith(".app")
           else os.path.realpath(client))
    approvals[key] = {
        "kScreenCaptureAlertableUsageCount": 1,
        "kScreenCaptureApprovalLastAlerted": now,
        "kScreenCaptureApprovalLastUsed": now,
        "kScreenCapturePrivacyHintDate": now + datetime.timedelta(days=3650),
        "kScreenCapturePrivacyHintPolicy": 315360000,
    }
path.parent.mkdir(parents=True, exist_ok=True)
path.write_bytes(plistlib.dumps(approvals, fmt=plistlib.FMT_BINARY))
PY
# The editor check switches light and dark through System Events. Its Automation row
# lives in the user's database, keyed by the guest agent's real path.
sqlite3 "$HOME/Library/Application Support/com.apple.TCC/TCC.db" "INSERT OR REPLACE INTO access (service,client,client_type,auth_value,auth_reason,auth_version,indirect_object_identifier_type,indirect_object_identifier,flags) VALUES ('kTCCServiceAppleEvents','$(realpath /opt/homebrew/bin/tart-guest-agent)',1,2,4,1,0,'com.apple.systemevents',0);"
killall replayd 2>/dev/null || true
sudo killall tccd 2>/dev/null || true
killall tccd 2>/dev/null || true
# A token that means nothing outside the VM, for the stand-in attn.
# A Keychain item made by `security` is closed to other code until its partition list
# names that code, and an ad hoc build is named by its cdhash. Without this, reading the
# token blocks on a Keychain prompt nobody can answer.
security delete-generic-password -s dev.attn.lens.sender -a default >/dev/null 2>&1 || true
security add-generic-password -A -s dev.attn.lens.sender -a default -w vm-stand-in-token
cdhash="$(codesign -dvvv "$HOME/Applications/Attn Lens.app" 2>&1 | sed -n 's/^CDHash=//p')"
security set-generic-password-partition-list -s dev.attn.lens.sender -a default -k admin \
    -S "apple-tool:,apple:,cdhash:$cdhash" login.keychain-db >/dev/null
# A bundle unpacked by tar is unknown to Launch Services until registered.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$HOME/Applications/Attn Lens.app"
