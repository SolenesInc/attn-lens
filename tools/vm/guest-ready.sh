# Runs inside the VM on every boot: leave nothing in front of the app under test.
osascript -e 'tell application "Terminal" to quit' 2>/dev/null || true
security unlock-keychain -p admin login.keychain-db
