# Building the Android app and putting it on the tablets

KitChat isn't in an app store: you install ("sideload") the APK on each phone and tablet. Download
`KitChat_<version>_android-universal.apk` from the repository's GitHub **Releases** ([releasing.md](releasing.md)) and
skip to [Install on a tablet or phone](#install-on-a-tablet-or-phone), or build it yourself as below. Use one or the
other for a device's updates unless your own builds use the same signing key as the releases.

## One-time setup on the PC

1. **Android Studio** (for its JDK and SDK manager). In **SDK Manager** install an Android SDK Platform (API 37), the
   **NDK (Side by side)** and **Android SDK Command-line Tools**. The SDK and an NDK are already in
   `%LOCALAPPDATA%\Android\Sdk`; what's missing on this PC is a JDK, which comes with Android Studio.
2. Environment variables (System Properties › Environment Variables), then open a new terminal:

   | Variable       | Value                                                      |
   | -------------- | ---------------------------------------------------------- |
   | `JAVA_HOME`    | `C:\Program Files\Android\Android Studio\jbr`              |
   | `ANDROID_HOME` | `%LOCALAPPDATA%\Android\Sdk`                               |
   | `NDK_HOME`     | `%LOCALAPPDATA%\Android\Sdk\ndk\<version>` (e.g. `30.0.16248370`) |

3. **Developer Mode** on in Windows (Settings › System › For developers): the Android build links the Rust library
   with a symlink, which Windows only allows in Developer Mode.
4. Rust's Android targets (already installed here):
   `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android`

## A signing key (once, and keep it safe)

Android only installs signed apps, and only lets an update replace the installed app if it's signed with the **same
key**. Make one key and keep it (and its password) somewhere safe, outside the repository:

```bash
keytool -genkeypair -v -keystore C:\Users\<you>\keys\kitchat.jks -keyalg RSA -keysize 4096 -validity 36500 -alias kitchat
```

Then create `src-tauri/gen/android/app/keystore.properties` (git ignores it):

```properties
storeFile=C:\\Users\\<you>\\keys\\kitchat.jks
storePassword=…
keyAlias=kitchat
keyPassword=…
```

## Build

```bash
npm install
npm run tauri android build -- --apk
```

The APK is `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`. It runs on
every kind of Android device. For a smaller file for modern tablets and phones only, add `--target aarch64`.

While working on the app, `npm run tauri android dev` runs it on a connected device or emulator with live reload (an
emulator reaches a hub on the PC at `10.0.2.2:8787`).

## Install on a tablet or phone

Either:

- **USB**: turn on Developer options and USB debugging on the device, plug it in, and run
  `adb install -r <the APK>` (`adb` is in `%LOCALAPPDATA%\Android\Sdk\platform-tools`).
- **Copy the file**: put the APK on the device (USB file transfer, Google Drive, …), open it in Files, and allow
  "Install unknown apps" for Files when asked.

Updates install the same way, over the old version, keeping messages and sign-in. Versions after 0.4.0 also
update themselves (below).

## Automatic updates

About twice a day KitChat checks the repository's GitHub Releases. When there's a newer version it downloads the APK
(only on Wi-Fi or another unmetered network), checks it against `SHA256SUMS.txt` and that it's signed with the same key,
and installs it the next time KitChat has been off screen for a minute, so it never closes in the middle of a message.

- **The first update on each device** asks to allow "Install unknown apps" for KitChat: tap the "A KitChat update is
  ready" notification and turn it on. On **Android 12 and newer**, later updates then install without asking.
- **Android 9–11** asks every time: tap the notification, then **Update**.
- An update ends KitChat's connection to the hub, like a restart does. A "KitChat was updated" notification asks to
  open it again; until then messages wait on the hub.
- It only installs releases signed with the key the device's KitChat has. A copy you built yourself with a different
  key (or a debug build) is left alone.

## First start on each device

1. Open KitChat › **Join my family**, and scan the code a parent shows (Family › QR icon next to the person), or type
   the hub address and the 6 letters.
2. Allow **notifications** when asked, and on Android 17 also **devices on your local network**.
3. A quiet "Connected to your family" notification stays while KitChat is running: that's what lets messages arrive
   while it's in the background. It can be hidden by long-pressing it and turning off "Staying connected" (message
   notifications still come).
4. Some phones (Samsung, Xiaomi, …) still stop background apps to save battery. In Settings › Apps › KitChat ›
   Battery, choose **Unrestricted**.

## Notes

- **Amazon Fire tablets** and other devices without Google Play services can't scan QR codes (the scanner is part of
  Play services). Type the code instead; everything else works.
- If Android ends KitChat while it's in the background (after a restart, or to free memory), messages arrive again
  the next time it's opened. No messages are lost; they wait on the hub.
- The same APK works on the parents' phones.
