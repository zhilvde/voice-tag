# 声签 VoiceTag

声签是一款 Windows x64 录音和转写工具。它支持录音、导入音频、离线识别、在线 API、手动听写，以及逐句校对和导出。

1.0.0 使用紫色主题与随应用分发的 Noto Serif SC 中文字体，界面动效支持系统的减少动态效果设置。 [字体许可](public/licenses/NotoSerifSC-OFL.txt)

## 安装与使用

1. 运行 `声签_1.0.0_x64-setup.exe`，在「安装位置」选择可写目录。安装包包含 Whisper 多语言模型和 WebView2 离线运行时。
2. 在「录音工作台」使用 Windows 默认麦克风录音，或导入 MP3、WAV、M4A、AAC、FLAC、OGG。最近录音可以从工作台直接打开，历史记录支持搜索、筛选、排序和重命名。
3. 在设置中选择转写方式。打开录音后选择识别语言并开始转写，也可以使用「手动听写」在当前播放位置添加文字。
4. 在转写结果中校对文字和说话人，复制全文或单个片段，也可以导出 TXT、Markdown。

录音和导入的音频文件默认保存在 Windows「文档」下的 `声签` 文件夹，可在「设置 → 通用」更改保存目录。项目、转写稿和设置存储在 `%APPDATA%\app.voicetag.desktop\voicetag.sqlite3`。备份时请先关闭声签，再备份项目数据目录和对应的音频文件。通过「设置 → 通用 → 卸载声签」卸载时，可保持「删除应用数据」未勾选以保留项目和设置。

## 转写方式

| 方式或模型 | 适用场景 |
| --- | --- |
| 内置离线识别 | 使用随安装包提供的 Whisper Base 多语言模型在本机 CPU 上识别，支持中文和英文。说话人可在转写后手动标注。 |
| Windows 离线识别 | 使用 Windows 已启用的桌面语音识别组件。可用语言取决于本机组件。 |
| 百炼 `paraformer-v2` | 根据音频区分说话人，可用于双人对话。重叠说话、噪声和相近声线仍可能影响分辨效果。 |
| 百炼 `qwen3-asr-flash-filetrans` | 异步转写并读取逐句时间戳。需要自动区分说话人时，可改用 `paraformer-v2`。 |
| 百炼 `qwen3-asr-flash` | 常规语音转写，应用会按接口上限自动分段，再按原录音时间合并结果。 |
| 兼容 OpenAI 的语音转写模型 | 通过 `/audio/transcriptions` 发送音频。时间戳和说话人标签取决于接口和模型。 |
| DeepSeek | 先在本机用内置模型转写，再将文字发送给模型校对。校对失败时会保留本机转写。 |

在「设置 → AI 配置」可保存多组服务 URL、模型名和 API Key。点击「使用」后，新转写会使用所选配置。API Key 由 Windows 加密保存在当前账户下。使用 `paraformer-v2` 或 `qwen3-asr-flash-filetrans` 时，音频会上传到百炼私有临时存储。

## 校对与导出

- **对话视图。** 说话人 A 和 B 分列左右并显示姓名首字头像。相邻的同一说话人片段组合在同一个气泡中，各片段仍保留时间戳，可单独编辑和定位。待确认和标记为重叠的发言单独显示。
- **播放定位。** 播放时高亮当前片段。可开关跟随播放，点击时间戳定位，或使用播放器前进和后退 5 秒。
- **说话人校对。** 用「重命名说话人」为同一录音中的 A、B 设置实际姓名。可单独调整片段的说话人，或拆分混在同一片段里的发言。
- **查找与替换。** 可按说话人筛选或搜索正文及姓名。`Enter` 跳到下一个匹配项，`Shift+Enter` 跳到上一个。批量替换支持预览，全局替换规则会按列表顺序应用于新转写稿。
- **复制与导出。** 复制全文会带上时间戳和说话人姓名。导出时可选择是否保留时间戳和姓名、是否合并相邻的同一说话人发言，以及是否仅导出选中的片段。

## 从源码运行和构建

在 Windows 上安装 Node.js 20.19+ 或 22.12+、Rust 1.89+ 的 MSVC 工具链、Visual Studio C++ 生成工具、Windows SDK、CMake 3.20+ 和 WebView2 运行时。首次拉取前端依赖、离线资源及打包工具时需要联网。

```powershell
npm ci
powershell -ExecutionPolicy Bypass -File scripts/prepare-offline.ps1
npm run tauri dev
```

发布前的检查和打包命令如下。

```powershell
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build -- --bundles nsis
```

默认安装包目录为 `src-tauri/target/release/bundle/nsis`。准备脚本会校验模型文件并构建离线程序。离线资源、依赖缓存和安装包由脚本或构建生成。 [whisper.cpp v1.8.3](https://github.com/ggml-org/whisper.cpp/tree/v1.8.3) · [Whisper Base Q5_1](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-base-q5_1.bin)

| 目录 | 内容 |
| --- | --- |
| `src/` | Vue 和 TypeScript 界面、主题及字体资源 |
| `src-tauri/src/` | Rust 桌面端、录音、转写、SQLite 和导出 |
| `src-tauri/inference/` | Whisper 离线识别程序及 Windows 语音识别脚本 |
| `scripts/` | 离线模型和识别程序的准备脚本 |
| `public/licenses/` | 随应用分发的字体许可证 |

## 版本与发布

从 `1.0.0` 起使用 `主版本.次版本.修订版本`。例如，兼容的问题修复使用 `1.0.1`，向后兼容的新功能使用 `1.1.0`，破坏公开行为或数据兼容性的升级使用 `2.0.0`。预发布版可使用 `1.1.0-beta.1`。 [Semantic Versioning 2.0.0](https://semver.org/lang/zh-CN/)

提交格式采用 `类型(范围): 描述`，例如 `feat(transcript): 支持按说话人筛选` 或 `fix(player): 修复拖动后高亮错位`。存在破坏性变更时使用 `!` 或在提交尾注中填写 `BREAKING CHANGE:`，说明影响及迁移方式。 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/zh-hans/v1.0.0/)

1. 在本地 `main` 分支合入通过检查的改动，写明用户可见的新增、变更和修复。 [CHANGELOG.md](CHANGELOG.md) · [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)
2. 同步 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock` 中的应用版本。锁文件的依赖版本按需要更新。
3. 重新检查并构建 Windows 安装包。验证升级后的数据读取，以及此次迭代涉及的操作路径。
4. 使用 `chore(release): 1.0.0` 形式提交发布改动，再用 `git tag -a v1.0.0 -m "Release 1.0.0"` 形式标记对应提交。后续发布替换为实际版本号。
