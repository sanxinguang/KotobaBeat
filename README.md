<p align="center"><img src="assets/icon.svg" width="80" height="80" alt="KotobaBeat" /></p>

# KotobaBeat

Windows 日语歌词伴侣。在听歌时查看同步歌词、中文翻译、汉字假名和逐句语法。

A Windows Japanese lyrics companion with synchronized lyrics, Chinese translations, furigana, and grammar notes. Built with Tauri 2, React, TypeScript, and Rust.

[使用方法](#使用方法) · [本地开发](#本地开发) · [歌词检索](#歌词检索) · [测试](#测试) · [常见问题](#常见问题) · [MIT 许可证](LICENSE)

## 功能

- **播放器联动**：通过 Windows GSMTC 读取歌曲、歌手、封面、播放状态和播放器提供的时间轴。
- **多源歌词**：网易云音乐、QQ 音乐、LRCLIB 并行检索；支持重新匹配、选择封面、导入 LRC 和粘贴歌词。
- **翻译与学习**：使用用户自己的 DeepSeek API Key，生成简体中文翻译、假名、N4–N2 语法、活用、省略和语气说明。
- **沉浸式歌词**：封面取色、缓慢流动的背景、当前行放大和渐隐滚动；可关闭流动背景、减少动画。
- **迷你窗口**：显示当前歌词、译文和简短语法，支持置顶和播放/暂停。
- **本地缓存**：保存已确认的歌词、封面选择、偏移和逐行解析；重新匹配不会清空已有翻译。
- **桌面集成**：托盘驻留、播放控制、播放器锁定和可选开机启动。

主窗口由专辑与播放控制、滚动歌词、当前句解析三部分组成。正常听歌时不显示接口诊断信息，也不会自动弹出候选确认窗口。

## 运行环境

| 项目 | 要求 |
| --- | --- |
| 系统 | Windows 10 1809+ / Windows 11，x64 |
| 界面运行时 | Microsoft Edge WebView2 Runtime |
| 播放器 | 向 Windows GSMTC 发布音乐信息的桌面播放器 |
| 在线功能 | 歌词检索需要网络；新翻译与解析需要有效 API Key 和服务余额 |

当前版本为 `0.1.0`，仅支持 Windows。播放器发布哪些信息取决于其版本和设置；开启 SMTC 不代表一定会提供可用时间轴。

## 使用方法

1. 启动播放器并播放日语歌曲。网易云需要在「设置 → 系统」开启 **SMTC**，然后暂停/播放或切歌一次。
2. 启动 KotobaBeat。找到可靠匹配或已确认的缓存时会直接显示歌词。
3. 在应用设置中填写 DeepSeek API Key，验证并保存后即可加载翻译和解析。
4. 如需更换歌词或封面，点击 **重新匹配**。歌词和封面可分别选择；无法匹配时可导入 `.lrc` / `.txt` 或粘贴歌词。

### 常用操作

- 点击歌词：查看该句解析；播放器支持跳转时，同时跳转到对应时间。
- 歌词上方的 `−` / `+`：每次调整 0.1 秒同步偏移，可一键重置。
- 「歌词显示假名」：控制滚动歌词注音，默认关闭；右侧解析默认显示注音。
- 「迷你歌词」：打开独立迷你窗。
- `Space`：播放/暂停。输入框、按钮和弹窗内不会拦截空格。
- `Esc`：关闭设置或弹窗。
- 关闭主窗口：应用继续驻留托盘；从托盘菜单退出才能完全结束进程。

### 歌词源联系信息

网易云、QQ 音乐和本地缓存无需填写此项。要使用 LRCLIB 和 MusicBrainz，请在「歌词源联系信息」中填写自己有权使用的公开项目 HTTPS 地址或邮箱。

该信息作为应用联系标识发给 **LRCLIB / MusicBrainz**，不用于登录，也不发送给网易云和 QQ。不要填写 API Key。未配置时跳过这两个服务，不影响其他歌词来源。

开发环境可设置后备值：

```powershell
$env:KOTOBABEAT_CONTACT_URL = "https://github.com/sanxinguang/KotobaBeat"
```

应用设置优先于环境变量；如要停用此配置，需要同时清空两处。

## 本地开发

### 前置依赖

- Node.js 22 或更新的兼容版本，以及 npm。
- Rust stable，使用 Windows MSVC 工具链。
- Visual Studio C++ Build Tools，安装「使用 C++ 的桌面开发」和 Windows SDK。
- WebView2 Runtime。

系统依赖安装说明见 [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows)。

```powershell
git clone https://github.com/sanxinguang/KotobaBeat.git
cd KotobaBeat
npm ci
npm run tauri dev
```

已有依赖时，日常启动只需执行 `npm run tauri dev`。项目可放在任意磁盘；Rust 构建目录默认位于 `src-tauri/target`，可能占用较大空间。

只预览界面可运行 `npm run dev`。浏览器预览使用虚构歌曲和演示解析，不读取系统播放器，也不调用翻译 API。完整桌面功能需要通过 Tauri 启动。

### 构建安装包

```powershell
npm run tauri build -- --bundles nsis
```

输出位置：

```text
src-tauri/target/release/kotobabeat.exe
src-tauri/target/release/bundle/nsis/KotobaBeat_0.1.0_x64-setup.exe
```

安装程序面向当前 Windows 用户；缺少 WebView2 时配置为下载其安装引导程序，因此首次安装可能需要网络。源码仓库不包含编译后的程序或安装包。

## 歌词检索

### 检索顺序

1. 优先使用本地已确认映射和缓存。
2. 未命中时并行查询已启用的网易云、QQ 音乐、LRCLIB。
3. 首轮缺少合理候选时，使用规范化歌名继续搜索；必要时通过 MusicBrainz 校准歌手、别名和录音版本。
4. 不确定的候选不会自动采用。用户可主动选择、导入歌词或继续使用此前确认的结果。

MusicBrainz 提供音乐元数据，**不是歌词源**。网易云与 QQ 读取接口没有面向本项目的稳定性保证，平台更新可能影响可用性，可在设置中独立关闭。

### 匹配依据

标题、歌手、专辑先进行 Unicode NFKC、全半角、空格和标点规范化；同时保留原始字段以及 Live、Remix、Acoustic、TV Size、翻唱、伴奏等版本信息。

| 依据 | 权重 |
| --- | ---: |
| 标题 | 36% |
| 歌手 | 22% |
| 时长 | 22% |
| 专辑 | 8% |
| 播放器来源 | 7% |
| 同步质量 | 5% |

自动采用需同时满足：总分 ≥ 0.84、标题 ≥ 0.92、歌手 ≥ 0.75、双方已知时长差 ≤ 8 秒、无版本冲突，且领先另一录音至少 0.07。缺失时长不会被推断值伪装成可靠证据；跨来源的相同录音不会造成虚假的竞争候选。

首轮约 4.5 秒预算，第二轮最多约 6 秒；各平台详情请求最多 4 个并发。一个来源超时不会丢弃其他来源已返回的结果。MusicBrainz 按每进程最多每秒一次限速，429/503 响应的 `Retry-After` 用于避免立即重复请求。

这些阈值是匹配规则，不是统计准确率保证。合成测试样本不等同于百首真实歌曲的线上评测。

## 翻译、假名与缓存

- 所有 API 请求由 Rust 发起。当前支持 `deepseek-v4-flash` 和 `deepseek-v4-pro`，具体可用性以服务账户为准。
- 请求附带整首歌词作为上下文，仅要求返回目标行；使用结构化 JSON 并校验行 ID。
- 首批优先处理当前附近的 3 行；之后每批最多 5 行，最多 2 个并发，完成一批就显示一批。
- 超时、缺失行或无效结果会有限重试；认证或余额错误会停止队列。假名局部异常不会丢弃有效译文。
- 切歌或重新选词会取消旧任务，提交结果时再次检查歌曲、歌词文档和模型，避免迟到结果覆盖当前内容。
- 行 ID 由文本和重复出现次数稳定生成。相同歌曲重新匹配、换来源或只调整时间戳时，已有解析可以复用。
- 缓存按歌曲、模型、学习范围、提示词版本、当前句及相邻句隔离。修改歌词会更新该句和相邻句；远处上下文变化不会强制重做整首。

翻译、假名和语法解释可能出错，适合辅助学习，不应代替教材或人工核对。

## 数据与隐私

- API Key 由当前 Windows 用户的 Credential Locker 保存，不写入 SQLite、项目文件或公开前端状态。
- 歌词、解析、偏移和设置保存在应用数据目录下的 `kotobabeat.sqlite3`，通常位于 `%LOCALAPPDATA%\com.local.kotobabeat`。
- 歌词检索会向所启用的来源发送歌曲信息；封面会从对应平台 CDN 下载。
- 在线翻译会将歌词上下文和学习要求发送给 DeepSeek，并消耗用户账户额度。
- 应用不录音、不读取播放器进程内存、不通过截图识别歌曲，不提供云端账户或云同步。
- 数据库不可用时会提示并进入临时模式，原文件不删除；临时模式的数据不能持久保存。

请勿把 API Key、个人缓存数据库、访问令牌或含私人信息的日志提交到仓库或 Issue。

## 项目结构

```text
src/
  App.tsx                 主窗口、候选选择与设置
  MiniApp.tsx             迷你窗口
  useSnapshot.ts          前端状态与播放时钟
  snapshotConnection.ts   事件订阅、版本校验与重连
  RubyText.tsx            假名标注
  lyricsMotion.ts         歌词滚动动画
src-tauri/src/
  lib.rs                  Tauri 命令与任务编排
  media.rs                Windows GSMTC 与播放控制
  lyrics/                 歌词解析、检索与匹配评分
  deepseek.rs             结构化翻译和语法请求
  analysis_jobs.rs        分批调度、取消与有限重试
  workflow.rs             任务有效性与设置校验
  db.rs                   SQLite 缓存
  credentials.rs          Windows 凭据库
scripts/verify.ps1         本地回归与打包入口
.github/workflows/ci.yml  Windows 自动测试
```

React 负责展示，Rust 负责系统接口、网络和数据，SQLite 负责缓存。无需额外 Node 服务、登录 Cookie 或代理后端。

## 测试

在已安装前置依赖的 Windows 环境中：

```powershell
npm ci
cd src-tauri
cargo fetch --locked
cd ..
powershell -ExecutionPolicy Bypass -File .\scripts\verify.ps1
```

脚本运行前端测试、TypeScript/Vite 构建、Rust 格式检查、严格 Clippy 和离线 Rust 测试。加上 `-Package` 可继续生成安装包。GitHub Actions 在 Windows 上运行相同的常规检查。

常规测试不访问真实歌词服务，不使用 API Key，也不产生翻译费用，覆盖：

- 标题规范化、版本识别、评分边界和 120 组合成匹配场景。
- LRC 时间戳、重复歌词、稳定行 ID、旧缓存迁移和损坏缓存处理。
- 本地 HTTP fixture 的成功、超时、401、429、503、格式变化和响应体大小限制。
- 分批重试、取消任务、切歌竞态、模型隔离及播放进度外推。
- 快照订阅、弹窗焦点、键盘操作、候选选择和界面错误恢复。

真实服务测试默认跳过，需手动执行：

```powershell
cd src-tauri
# 本地性能测量，不联网
cargo test measure_warm_cache_latency -- --ignored --nocapture
# 读取当前 Windows 音乐会话
cargo test inspect_live_windows_media_sessions -- --ignored --nocapture
# 歌词源连通性；公共源需上述联系信息环境变量，测试不读取应用设置
cargo test inspect_live_lyrics_providers -- --ignored --nocapture
# 使用已保存的 Key 翻译三句测试文本，会产生少量 API 费用
cargo test inspect_live_small_analysis_batch -- --ignored --nocapture
```

## 常见问题

### 播放器正在播放，但没有识别到歌曲

检查播放器是否开启系统媒体控制。网易云需要开启 SMTC 后再暂停/播放或切歌。锁定的播放器退出后，可以切回「自动跟随播放器」。不是所有客户端版本都发布 GSMTC 数据。

### 拖动播放器进度后，歌词没有跟着跳

只有播放器提供有效时间轴时才能校正跳转。进度前的 `≈` 表示本地估算；此时无法可靠感知播放器中的拖动，也不能保证 500ms 的同步精度。

### 重新匹配会重新收费翻译吗

不会因为重新匹配而清空已有解析。相同歌词会使用缓存；歌词内容、相邻句、模型或提示词版本变化时，相关行可能需要重新请求。

### 歌词匹配不到或翻译没加载完

可手动重新匹配、导入 LRC 或粘贴歌词。翻译失败时先检查网络、Key 和账户余额；重试仅补齐缺失行。

### 窗口关闭了，程序还在运行

主窗口关闭后会驻留托盘。完全退出请使用托盘菜单；开发时不要重复启动多个实例。若 Vite 提示 1420 端口占用，先检查已有开发进程。

## 已知限制

- 当前只实现 LRC 和纯文本歌词，不包含加密 QRC 解码。
- 没有完整的无播放器手动搜歌模式；导入歌词需要先识别到歌曲。
- 网易云和 QQ 接口可能因平台调整、限流或网络条件而失效。
- 同时运行多个主进程不是推荐用法，请使用应用内迷你窗。
- 尚未完成全部播放器版本、DPI/分辨率、全新系统安装/卸载及百首真实歌曲的验收。

## 参与与授权

问题反馈请使用 [Issues](https://github.com/sanxinguang/KotobaBeat/issues)，说明系统版本、播放器版本和复现步骤。提交代码前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

本项目使用 [MIT License](LICENSE)。歌词、封面及第三方依赖仍受各自版权、许可证和服务条款约束；本项目许可证不授予这些第三方内容的使用权。仓库中的演示歌词为虚构测试内容，不随源码分发真实歌曲歌词。
