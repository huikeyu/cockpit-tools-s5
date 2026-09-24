# 从纯源码构建 Cockpit Tools Isolation V4

本仓库基于上游 `jlcodes99/cockpit-tools` tag `v1.3.59`。公开源码包采用单顶层目录结构，不包含 `node_modules`、`.toolchain`、`.bootstrap`、`target`、`dist`、编译 EXE、运行日志、账号数据或真实代理/订阅凭据。

## Windows x64 一键构建

1. 安装 Visual Studio 2022 Build Tools，并勾选“使用 C++ 的桌面开发”和 Windows SDK。
2. 解压源码 ZIP，保留完整目录结构，确保可以连接 Node.js、Go、Rust、Xray-core 与 sing-box 的官方下载地址。
3. 在源码根目录双击 `build.cmd`。脚本下载并校验固定版本的 Node.js、Go、Rust、Xray-core 和 sing-box，安装 npm 依赖，执行 TypeScript、Rust 代理解析和 Go 网关测试，然后构建 NSIS 安装程序。
4. 构建过程使用本地模拟代理运行成品 `--proxy-self-test`。通过后在 `artifacts/` 生成一个 `V4-1.3.59-win-x64-时间戳.zip`；该交付 ZIP 中只有安装 EXE。构建日志与自检报告留在本机 `build-logs/` 和 `artifacts/时间戳/`，不会放进交付 ZIP。

源码包没有随附 Xray 或 sing-box 可执行文件；首次构建会从官方发布页下载并核对 SHA-256。`src-tauri/proxy-core/` 保留许可证和 sing-box v1.14.1 对应源码归档。不要把你自己的 `.codex`、Cockpit 账号目录、代理链接、订阅令牌或构建日志提交到公共仓库。

## 验证与开发

在 Windows 上完成依赖准备后，可运行：

```powershell
npm ci
npm test
npm run typecheck
cargo test --locked -p cockpit-account-proxy
cd sidecars/cockpit-cliproxy
go test ./...
```

`build.cmd` 会自行配置共享工具链和缓存；上述手工命令需要相应工具已加入 `PATH`。Hysteria2/TUIC 使用独立 sing-box 进程，VLESS/HTTP/SOCKS5 使用 Xray。两个内核都只绑定账号专属的 `127.0.0.1` 本地端口。

## 数据和网络边界

- V4 沿用隔离版 V2 的 Cockpit 数据目录 `%LOCALAPPDATA%\CockpitTools-Isolation\V2`；与 V2/V3 不要同时运行。
- 默认安装的 ChatGPT/Codex 桌面客户端使用系统原有 `CODEX_HOME`（未设置时为 `~/.codex`）。原版 Cockpit 与本版不要同时切换或管理它。
- API 服务按实际选中的账号使用该账号的代理；代理组全部故障时阻断请求，不改走全局代理或直连。切换期间已经失败的请求不会自动重放。
- 网页登录和 Token 导入先选代理。登录过程中锁定当前线路，故障时需要取消并重新登录，避免中途变更公网出口。
- 订阅导入一次性解析 V2Ray Base64 或 Clash YAML 的受支持节点。拉取时须选择库存代理，或显式允许本机网络访问订阅服务；后者可能暴露原始出口 IP。
- 账号池代理不接管桌面客户端的登录、更新、插件等非模型流量。

详情见 [V4 网络与功能说明](V4-ISOLATION-README.md)。本项目源代码继承上游 **CC BY-NC-SA 4.0** 非商业限制；参见 [LICENSE](LICENSE)。内置 sing-box 按其独立 GPLv3-or-later 条款分发。
