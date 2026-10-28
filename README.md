# sideload

Keep a sideloaded iOS app signed with a free Apple ID, without AltStore.

Xcode issues the provisioning profile through its own Apple ID login, `codesign` re-signs the IPA, and `devicectl` installs it. A launch agent re-signs the app before the 7-day profile expires.

## Requirements

- macOS with Xcode, signed in under Settings > Apple Accounts
- An iPhone paired with this Mac, with Developer Mode on, reachable by USB or the same Wi-Fi

## Usage

```sh
cargo install --path .
sideload install App.ipa --team <team-id>
sideload agent
```

`install` signs the app as `<bundle-id>.<team-id>` and installs it. `agent` loads a launch agent that runs `sideload refresh` every hour. When less than 3 days are left, it fetches a fresh profile and reinstalls the app. App data survives every reinstall.

`sideload status` shows the expiry. `sideload refresh --force` re-signs now.

The team ID is the `OU` of your development certificate:

```sh
security find-certificate -c "Apple Development" -p | openssl x509 -noout -subject
```

App extensions are removed, because a free team can create only 10 App IDs per week. State and logs are in `~/Library/Application Support/sideload`.
