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

已用这把密钥签过、并登记进 `src-tauri/src/receiver.rs` 审计表的版本：

| Receiver | APK SHA-256 | 说明 |
| --- | --- | --- |
| 0.11.0 / code 14 | `9f5234bebd1c9728b24b557d8ed84f7fc82dcf66129d01588e294b22563d2cba` | 发布流程验证包（PC 1.4.0 从脏树构建，未对外发布） |
| 0.12.0 / code 15 | `5a29fc35e9ac065b57744abef13d8d06996a8554a5a0c17277bc425a58fac2e9` | 首个对外发布签名版本，随 PC 1.5.0 |


以下为同签名历史及本地候选记录；各 PC 版本的接受范围以其自身审计表为准：

| Receiver | APK SHA-256 | 说明 |
| --- | --- | --- |
| 0.17.5-v3-flow-candidate / code 39 | `a77bf771a07272bba5d7437e1d7dd69906387e256537350de990eb789fc2b0d0` | UI/UX 阶段 0-1（术语、单一确认、折叠区）；source 01f16af（sourceModified=false）；未进行设备安装验证 |
| 0.17.4-v3-flow-candidate / code 38 | `01beca356daf6e5630fbce9ff5c24a3b61f5ca75b08f22ce40c1ab08a29037c7` | PC 1.10.5 本地候选；source f1040de（sourceModified=false）；未进行设备安装或原地升级验证 |
| 0.17.3-v3-flow-candidate / code 37 | `379d66041630c65f2ecfab2dedbf649441687759bfdd5d41746079be0fb4dbac` | 本地候选；source 1739473（sourceModified=false）；未进行设备安装或原地升级验证 |
| 0.17.2-v3-flow-candidate / code 36 | `0f34c03b5618182fcd90f5a33883bd7a6de5f5a4696d9c1a64603f3f4983194e` | 本地候选；source ce81f03（sourceModified=false）；未进行设备安装或原地升级验证 |
| 0.17.1-v3-flow-candidate / code 35 | `8f8e76e4b9e2dc1e3bb8d4575f5e21e77838056b2b55536c5736eedcccf4f15f` | PC 1.10.1 USB Direct crash-fix 候选；source 2bd998d（sourceModified=false）；未进行设备安装或原地升级验证 |
| 0.17.0-v3-flow-candidate / code 34 | `711c20c4348b1865d0475244271271f1fc32d70888f1f77054481bba138fed83` | PC 1.10.0 本地 USB Direct 候选；source 8ca9638（sourceModified=false）；未进行设备安装或原地升级验证 |
| 0.16.0 / code 33 | `c05f3407aefd7e324017ccfecd5fd431d8eb1e1e9beac4b66e4e59b4cbc0647d` | 主屏状态与文案简化；基于 c299c80 未提交源码验收构建（sourceModified=true），随 PC 1.9.0 |
| 0.13.0 / code 16 | `4f23da449cf0340effe76c9ec90b0931a78ac688c45f21dab314825a410de9ee` | 首个 AOD 构建（同版本覆盖失败，仅存档） |
| 0.13.0 / code 16 | `b2d501f8a387e1b339dea1f40d153f4beb00545a3f5d8597da6b0c686ca14d35` | AOD 修复版（底栏、波形、通知重开） |
| 0.13.1 / code 17 | `c3d56258a0ffa94dae4a861affb21ff5783f53b7a1c22813eb8be47b0e14313b` | 闪退修复（负数 padding） |
| 0.13.2 / code 18 | `1daf7889d1330d61187c938d2b3591029ff5c8308982e288c0fd5590c10fe732` | 防烧屏漂移修复 |
| 0.13.6 / code 22 | `1b73f76b98ae2470969531085fc5bd7d5114055a58c5df3beeb0f6ad6602c549` | 波形逐段绘制、Recv 稳定、时钟放大；随 PC 1.6.6 |
| 0.13.7 / code 23 | `ff762492a15c746de025cc84e877354a567dc31d747dbf857e08b9ef03ae189d` | 波形下移、中心向外传播与连续正弦曲线；本地未提交源码验收构建，随 PC 1.6.7 |
| 0.14.0 / code 24 | `ce25dab8fc20891b3947369b4165bd2cff89388b9d7c9d99583605addecd4c09` | 波形增强、系统亮度与右侧音量手势；本地未提交源码验收构建，随 PC 1.7.0 |
| 0.15.0 / code 25 | `469b4ef9ec4fd441c1e2cd64ab6423316df402b9f3f6aac0e43442309308662c` | 弱信号波形增强与两端渐隐；本地未提交源码验收构建，随 PC 1.8.0 |
| 0.15.1 / code 26 | `a6b7c61b6ad313ed47a9c817ae997f4a4be9f8045bb0053c7a38b288da33272d` | 波形扩大到时钟与底栏之间的可用区域；本地未提交源码验收构建，随 PC 1.8.1 |
| 0.15.2 / code 27 | `c9593516c9ec9e81f2f533d8075884f35ab35bd8e501d7fe7ab2175fc4775219` | 振幅回收至 2/3、漂移减半、常显跨播放模式续存、音量手势去延迟；与无线链路修复配套，本地未提交源码验收构建，随 PC 1.8.2 |
| 0.15.3 / code 28 | `e15777fdf1c448b404dec9b7216ea8d6b8ff6c37196fdccbae089cae0336e4bd` | 双线波形改为上方左声道/下方右声道并加 L/R 标记；配套 PC 1.8.3 直接启动前台服务的尝试因服务未导出而失败，仅存档 |
| 0.15.7 / code 32 | `a103232b24e1fcddb456d3d8b986ca69ec902f147c8d1665aecca1cda189ec5c` | 时钟/波形/指标栏整体漂移（±16dp × ±9dp）与“退出”闲置淡出；干净提交 fc529c9 构建 |
| 0.15.6 / code 31 | `c80a965f525c81618a096a3a684e7cfe67cfdc201f9d50eb9006560fd3cc0b72` | 1.8.6 发布构建：干净提交 f7b4576 构建（sourceModified=false），AOD 预览 + 无线/播放修复 |
| 0.15.5 / code 30 | `e70c53d1393de3a976becf78d8ab5c3d4b94bcdf5be890e0901aa79fd2e8314e` | 无线监听/发现套接字移出主线程（修复换网络后开始接收异常）、右声道反相；本地未提交源码验收构建，随 PC 1.8.5 |
| 0.15.4 / code 29 | `630e24796fc7c5fc07dc78db5ec39a29a84fcfc3bf658fc50037d5e3d283930e` | BootstrapActivity 改为无窗口转发（不再白色页）、双线波形回到同一基线；本地未提交源码验收构建，随 PC 1.8.4 |

每次发布新 APK 都要追加一行并同步审计表，否则下一版无法识别手机上的旧版本。

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
