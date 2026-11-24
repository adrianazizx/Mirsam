## Download Mirsam {{VERSION}}

| Your computer | Download |
|---|---|
| Mac (Apple silicon or Intel, macOS 13 or later) | [Mirsam-macOS.dmg](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-macOS.dmg) |
| Windows 10 or 11 (64-bit) | [Mirsam-Windows-Setup.exe](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-Windows-Setup.exe) (or the [.msi](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-Windows.msi) for managed installs) |
| Ubuntu, Debian | [Mirsam-Linux-x86_64.deb](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-Linux-x86_64.deb) |
| Fedora, openSUSE | [Mirsam-Linux-x86_64.rpm](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-Linux-x86_64.rpm) |
| Any other Linux | [Mirsam-Linux-x86_64.AppImage](https://github.com/{{REPO}}/releases/download/v{{VERSION}}/Mirsam-Linux-x86_64.AppImage) |

### Installing

<!-- macos:signed -->
- **Mac:** open the disk image and drag Mirsam to Applications.
<!-- /macos:signed -->
<!-- macos:unsigned -->
- **Mac:** open the disk image and drag Mirsam to Applications. This build is not yet signed by
  Apple, so the first time you open it macOS says it cannot check it for malicious software. Choose
  **Done**, then open **System Settings → Privacy & Security**, scroll down, and choose **Open Anyway**.
<!-- /macos:unsigned -->
<!-- windows:signed -->
- **Windows:** run the setup program. It is signed by the SignPath Foundation for the Mirsam
  project. A new release can still show "Windows protected your PC" for a few days, until
  SmartScreen knows it: choose **More info**, check that the publisher is SignPath Foundation,
  then **Run anyway**.
<!-- /windows:signed -->
<!-- windows:unsigned -->
- **Windows:** run the setup program. Windows may show "Windows protected your PC" because the
  installer is not yet signed: choose **More info**, then **Run anyway**.
<!-- /windows:unsigned -->
- **Linux:** install the package with your software installer, or make the AppImage executable
  (`chmod +x Mirsam-Linux-x86_64.AppImage`) and run it.

`SHA256SUMS.txt` lists each file's SHA-256 checksum. How the installers are signed, and by whom:
[Code signing policy](https://github.com/{{REPO}}/blob/main/docs/CODE_SIGNING_POLICY.md) ·
[Privacy](https://github.com/{{REPO}}/blob/main/docs/PRIVACY.md).

### التثبيت

<!-- macos:signed -->
- **ماك:** افتح ملف القرص واسحب «مرسام» إلى مجلد التطبيقات.
<!-- /macos:signed -->
<!-- macos:unsigned -->
- **ماك:** افتح ملف القرص واسحب «مرسام» إلى مجلد التطبيقات. هذا الإصدار غير موقَّع من Apple بعد،
  لذلك يُظهر النظام تحذيرًا عند فتحه أول مرة. اختر **تم**، ثم افتح **إعدادات النظام ← الخصوصية والأمن**،
  ومرِّر إلى الأسفل واختر **الفتح على أي حال**.
<!-- /macos:unsigned -->
<!-- windows:signed -->
- **ويندوز:** شغِّل برنامج الإعداد. المثبِّت موقَّع من مؤسسة SignPath لمشروع «مرسام». قد يظهر تنبيه
  «Windows protected your PC» في الأيام الأولى بعد صدور الإصدار: اختر **More info**، وتأكد أن الناشر
  SignPath Foundation، ثم اختر **Run anyway**.
<!-- /windows:signed -->
<!-- windows:unsigned -->
- **ويندوز:** شغِّل برنامج الإعداد. قد يظهر تنبيه «Windows protected your PC» لأن المثبِّت غير موقَّع
  بعد: اختر **More info** ثم **Run anyway**.
<!-- /windows:unsigned -->
- **لينكس:** ثبِّت الحزمة من مدير البرامج، أو اجعل ملف AppImage قابلًا للتشغيل ثم شغِّله.
