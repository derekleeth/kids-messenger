# Making a release

The **Release** workflow on GitHub builds everything KitChat ships. Pushing a version tag publishes the files as a
GitHub Release. The repository is public, so anyone with the link can download them; they contain no family data.

| File                                         | What it's for                                               |
| -------------------------------------------- | ----------------------------------------------------------- |
| `KitChat_<version>_x64-setup.exe`            | The Windows desktop app ([desktop.md](desktop.md))          |
| `KitChat_<version>_android-universal.apk`    | Phones and tablets ([sideload.md](sideload.md))             |
| `kitchat-hub_<version>_windows-x64.exe`      | The hub on a family PC ([hub_setup.md](hub_setup.md))       |
| `kitchat-hub_<version>_linux-x86_64.tar.gz`  | The hub on an x86-64 Linux VPS ([vps_setup.md](vps_setup.md)) |
| `SHA256SUMS.txt`                             | Checksums of the files above                                |

## Once: give GitHub the Android signing key

Android only accepts an update signed with the same key as the installed app, so a tagged release won't build without
it. Make the key as in [sideload.md](sideload.md) if you don't have one, then, from this repository in PowerShell:

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("$env:USERPROFILE\keys\kitchat.jks")) | gh secret set ANDROID_KEYSTORE_BASE64
```

```powershell
gh secret set ANDROID_KEYSTORE_PASSWORD
```

```powershell
gh secret set ANDROID_KEY_ALIAS
```

```powershell
gh secret set ANDROID_KEY_PASSWORD
```

The last three ask for the value. Keep your own copy of the key and passwords: GitHub won't show them again.

If the devices have KitChat from a build signed with a different key, the first release won't install over it. Remove
the old app and install the new one. Removing it deletes what's stored on that device, so check that the device's
messages are on the hub first; afterwards the device joins the family again with a new code.

## Each release

1. Set the same new version in `package.json`, `src-tauri/tauri.conf.json` and `Cargo.toml` (`[workspace.package]`).
   Run `npm install` and `cargo check -p kitchat-hub` so both lockfiles follow, and commit.
2. Tag that commit and push the tag:

   ```powershell
   git tag v0.4.0
   ```

   ```powershell
   git push origin v0.4.0
   ```

Within about half a day, phones and tablets with a KitChat newer than 0.4.0 download the new APK and install it
([sideload.md](sideload.md#automatic-updates)). Draft and pre-release releases are skipped.

The workflow stops without publishing if a build or test fails, the tag doesn't match the version, or the signing
secrets are missing. After fixing a problem that didn't need a code change, re-run the workflow; a re-run replaces the
release's files. A code fix needs a new version and tag.

## Trying the builds without releasing

Actions › **Release** › **Run workflow**. The files are attached to the run and nothing is published. Without the
signing secrets the APK is named `…-TEST-KEY.apk`: it's signed with a throwaway key, so don't install it on the family's
devices.
