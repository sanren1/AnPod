# Receiver 发布签名

预览包一直使用候选 debug 证书。debug 证书只适合测试：它由本机生成、没有备份流程，一旦
丢失，后续版本无法覆盖安装。对外发布必须换成独立保管的发布密钥。本文只涉及 Android
Receiver；PC 端 `AnPod.exe` 是免安装便携程序，不做代码签名。

## 当前发布证书

| 项 | 值 |
| --- | --- |
| 主体 | `CN=AnPod Receiver, O=AnPod, OU=Release` |
| 算法 | RSA 4096 / SHA-256，自签 |
| 有效期 | 至 2051-10-17 |
| 证书 SHA-256 | `f7782237850a0a84b44bf66b53514238fb83a43e0d7e737c58369bfd72574573` |
| 密钥别名 | `anpod-receiver` |

证书 SHA-256 就是 `apksigner verify --print-certs` 打印的
`Signer #1 certificate SHA-256 digest`，也是 `receiver.json` 里的 `signerSha256`。

## 密钥放在哪里

密钥和口令**不能进仓库**（`.gitignore` 已排除 `*.p12`、`*.jks`、`*.keystore`）。生成时
选定一个仓库外的私有目录，目录里应有四个文件：

| 文件 | 用途 |
| --- | --- |
| `anpod-receiver-release.p12` | PKCS#12 密钥库（私钥 + 证书），真正的发布密钥 |
| `anpod-receiver-release.cer` | 只含证书，可公开分发 |
| `anpod-receiver-release.env.txt` | 四项 `ANPOD_RECEIVER_RELEASE_*` 值 |
| `signer-cert-sha256.txt` | 证书 SHA-256，便于和 `receiver.json` 对照 |

新机器或新密钥轮换时，用 JDK 17 的 keytool 生成（构建 Receiver 本来就需要 JDK）：

```powershell
$pass = -join (1..40 | ForEach-Object { 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'[(Get-Random -Maximum 62)] })
keytool -genkeypair -v -keystore anpod-receiver-release.p12 -storetype PKCS12 `
    -storepass $pass -keypass $pass -alias anpod-receiver `
    -keyalg RSA -keysize 4096 -sigalg SHA256withRSA -validity 9131 `
    -dname "CN=AnPod Receiver, O=AnPod, OU=Release"
keytool -exportcert -alias anpod-receiver -keystore anpod-receiver-release.p12 `
    -storepass $pass -file anpod-receiver-release.cer
keytool -list -keystore anpod-receiver-release.p12 -storetype PKCS12 -storepass $pass
```

`keytool -list` 打印的 `Certificate fingerprint (SHA-256)` 就是 apksigner 的
`Signer #1 certificate SHA-256 digest`，也是 `receiver.json` 的 `signerSha256`；把它写进
`signer-cert-sha256.txt`。

不要用 .NET 的 `X509Certificate2.Export(Pkcs12)` 生成：它写入的 friendlyName 不会被 Java
当作 key alias（实际读出来是 `1`），Gradle 会报
`No key with alias 'anpod-receiver' found`。已经这样生成的话，用
`keytool -importkeystore -srcalias 1 -destalias anpod-receiver` 重写一次即可，证书与私钥不变。

## 构建与打包

```powershell
# 1. 只在当前 shell 注入密钥口令，不从仓库读取
Get-Content <密钥目录>\anpod-receiver-release.env.txt |
    Where-Object { $_ -match '^ANPOD_' } |
    ForEach-Object { $name, $value = $_ -split '=', 2; Set-Item -Path "env:$name" -Value $value }

# 2. 构建发布签名的 Receiver（会自己用 apksigner 校验并把摘要写进 receiver.json）
packaging\build-receiver.ps1 -Variant release -JavaHome <JDK17> -AndroidSdkRoot <SDK> `
    -OutputDirectory artifacts\release-receiver-<版本>

# 3. 打包便携包（包含 release APK 与 build-metadata.json）
packaging\publish-tauri.ps1 -Version <版本> -IncludeReceiverRelease `
    -ReceiverArtifactDirectory artifacts\release-receiver-<版本>
```

`build-receiver.ps1` 在缺少任一 `ANPOD_RECEIVER_RELEASE_*` 时会直接失败，不会退回 debug
签名；`publish-tauri.ps1` 的 `-IncludeReceiverRelease` 会拒绝 debug 变体、拒绝候选 debug
证书，并用 aapt/apksigner 复核版本、targetSdk 和签名者。

发布包里的 `build-metadata.json` 会记录 `workingTreeDirty`。为 `true` 表示该包是用未提交的
工作树构建的，只能用于验证流程，不能对外发布；正式包必须从干净提交构建。

CI 若要出正式包，把四项值放进受保护的 secrets（如 `ANPOD_RECEIVER_RELEASE_STORE_BASE64`
加解密后落盘，或直接用 runner 的临时目录），不要打印任何口令。

## 每次发布后要做的事

Receiver 的升级校验只认两种 APK：当前包内 `receiver.json` 的摘要，或
`src-tauri/src/receiver.rs` 的 `AUDITED_RECEIVER_ARTIFACTS` 里登记过的摘要。所以**每发布
一个 APK，就要把它登记进审计表**，否则下一版无法识别“手机上装的上一版”，会报
`RECEIVER_UPDATE_REQUIRED`。

登记内容：`sha256`、`version_code`、`version_name`、`source_commit`、`signer_sha256`。
构建脚本已经把它们写进 `receiver.json`，直接抄即可。

## 切换签名的代价

Android 不允许跨签名覆盖安装。debug 预览包 → release 包、或轮换发布密钥，都会让
`adb install -r` 失败（`INSTALL_FAILED_UPDATE_INCOMPATIBLE`）。程序不会自动卸载，用户必须
手动卸载 Receiver，配对绑定随之清空，需要重新走一次 USB 配对。因此：

1. 第一次发布签名版本时，在发布说明里写清这一步。
2. 切换后不要再用旧的 debug 包做升级测试。
3. 密钥轮换等同一次强制迁移，非必要不做。

## 备份与轮换

- 至少两份离线备份（移动盘 / 加密归档），与口令分开存放；`README.txt` 里有同样的提醒。
- 备份后做一次恢复演练：用备份的 `.p12` 签一个测试 APK，确认 apksigner 通过。
- 轮换步骤：生成新密钥 → 构建新版 → 在发布说明中要求卸载重装 → 把旧 APK 摘要保留在
  审计表里（用于识别旧安装），新 APK 摘要加入审计表。

## 与候选包的边界

- `-ReviewPackage` / `-ReviewWorkingTree` 仍然走候选 debug 证书，只用于内部评审。
- `-IncludeReceiverRelease` 与 review 开关互斥；同时传入会被脚本拒绝。
