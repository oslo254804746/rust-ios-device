# rust-ios-device 稳定性任务最终进度记录

> 复审日期：2026-09-07
>
> 复审起点：`d794fd75219b1ef40df818fc4ed7b89b9d320eb3`
>
> workspace version：`0.1.14`

本记录汇总 GLM/Gemini 交付后的复审、必要修复和最终验证。结论表示任务指定范围内的实现和本机可行验证已经完成；跨平台 runner、真机、管理员权限和真实发布仍按下文记录为 NOT_RUN。

## 最终结论

| 任务 | 状态 | 结论 |
|---|---|---|
| T1 初始化期限 | DONE | H2/XPC/RSD 指定初始化入口使用受控期限；共享预算不重置，RSD queued→legacy→passive 结构保持。 |
| T2A 发布门禁 | DONE | release、crate、wheel、sdist 发布节点接入必需门禁；静态检查器和负向回归测试通过。 |
| T2B 产物 smoke | DONE（Windows 本机） | CLI/FFI 归档、动态/静态库、C smoke、wheel/sdist 在上传前的检查逻辑已实现并完成 Windows 验证。 |
| T3 kernel feature | DONE | `ios-py`/`ios-ffi` 独立依赖显式包含 `tunnel-kernel`；mode 合同和最终 wheel smoke 通过。 |
| T4 取消复用 | DONE | 半帧读写、XPC body、跨 H2 帧发送取消均取得决定性证据；可恢复边界持久化，不能安全恢复的边界要求重连。 |
| V 收尾验证 | DONE（Windows 宿主侧） | 下列最终命令和产物验证通过。 |

## 复审修复

- 在既有 H2 header/payload 和未完成写入持久化基础上，`crates/ios-core/src/xpc/h2_raw.rs` 补充重复取消/WriteZero 帧尾恢复、DATA 流量窗口预留和 DATA/HEADERS/WINDOW_UPDATE 控制边界状态，禁止重复发送或错误恢复。
- `crates/ios-core/src/xpc/rsd.rs` 在 XPC header 消费后保持 body 读取标记；取消、长度校验失败或 body I/O 错误均要求重新建立连接。XPC 整条消息跨多个 H2 DATA 帧发送时，取消也会 poison 连接，后续 send/recv 和缓存路径不能绕过该状态。
- `crates/ios-core/src/xpc/client.rs` 在父提交已修正未 poll 死锁的基础上，补充 5 秒 watchdog、oneshot 错误检查和迟到回复等待保护，并保留“请求完整发送后等待回复时取消可复用”的合同。
- `crates/ios-core/src/device/tests.rs` 补充 RSD SETTINGS 停顿/分片、queued 成功及 queued→legacy、legacy→passive fallback 回归。
- `.github/workflows/ci.yml` 和 `scripts/check-release-gates.py` 补齐门禁拓扑、归档文件断言、动态/静态库检查、C/Python smoke、wheel/sdist 单产物检查；门禁检查器的 7 项回归测试通过。
- 复核 `crates/ios-py/Cargo.toml`、`crates/ios-ffi/Cargo.toml` 已显式启用 `mdns`、`tunnel-kernel`、`tunnel-userspace`；本轮将 `crates/ios-ffi/README.md` 的旧 feature 说明同步为 kernel TUN 也可用。
- `crates/ios-core/tests/xpc_send_cancellation.rs` 用公开 `XpcClient` 验证跨多个 H2 DATA 帧发送取消：基线后续 send 返回 `Ok(())` 且 wire 计数增加，修复后 send/recv 均要求重连且计数不增加。

## 最终验证证据

| 命令或产物 | 结果 |
|---|---|
| `cargo test -p ios-core --all-features --locked` | 971 passed，0 failed，1 ignored。 |
| `cargo test --workspace --exclude ios-core --exclude ios-py --locked --quiet` | 523 passed（510 CLI、13 FFI）。 |
| `cargo test -p ios-py --no-default-features --locked --offline` | 6 passed；host 测试未启用 packaging-only `extension-module`。 |
| `cargo test -p ios-ffi --locked --offline` | 13 passed。 |
| `cargo fmt --all -- --check`、`git diff --check` | 通过。 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 通过，零警告。 |
| `cargo check -p ios-core --no-default-features` 及 `classic/developer/ios17/management` | 五组通过；子集中的既有 dead-code 警告不扩大解释为全配置零警告。 |
| `cargo +1.80 check --workspace --exclude ios-py --locked` | Rust 1.80.1 工具链通过；该命令按约定不包含 `ios-py`。 |
| `uv run --with pyyaml python scripts/check-release-gates.py`、`scripts/test_check_release_gates.py` | 检查器和 7 项回归测试通过；坏门禁、失败条件、`always()` 与 `continue-on-error` 变体均被拒绝。 |
| `cargo tree --locked --offline -e features -p ios-py` / `-p ios-ffi` | 两个绑定均显示 `tunnel-kernel`、`tunnel-userspace` 和 `tun-rs v2.5.1`。 |
| Windows CLI/FFI release + C smoke | 归档解包、CLI `--version`/`--help`、clang/MSVC 编译链接和运行通过；C smoke 验证无设备 NULL 参数合同及 DLL 来自解包目录。 |
| Windows wheel | `rust_ios_device_tunnel-0.1.14-cp39-abi3-win_amd64.whl`，4,470,546 bytes，SHA-256 `7EA65E8D22B91071961F0DE021332ED6D7DE091FF4B38A6D43D46D23BAD3E4C0`；干净 Python 3.13 venv 中 import、callable 和非法 mode `ValueError` 合同通过。 |
| Python sdist | `rust_ios_device_tunnel-0.1.14.tar.gz`，637,985 bytes，SHA-256 `8F601C3BA77C2261F5A5EF6F136AEC77EB7F83B0255E0386CC8C98FD833E5ED6`；解包包含 ios-py/ios-core Cargo 配置、lockfile、XPC 修复源码和 `sending_message_in_progress`。 |

构建环境为 Windows `x86_64-pc-windows-msvc`，本轮实际默认 Rust/Cargo 为 1.90.0，绑定 smoke 使用 uv 管理的 CPython 3.13.5。详细命令输出位于 `target/stabilization-review-2026-09-07/`，该目录为本机验证工作区，不作为发布凭据或发布结果。

## NOT_RUN 与范围边界

- Linux、macOS、aarch64 runner 的实际构建、归档安装和运行未在本机执行，不能由 Windows 结果替代。
- 未操作真实 iOS 设备、pair record、管理员权限 TUN 激活或真实服务业务流程。
- 未创建真实 release tag，未触发 tag-only release/wheel/sdist workflow，未执行 crates.io、PyPI、GitHub Release 发布，也未访问发布凭据。
- 本轮 T1 只覆盖任务指定初始化入口；`pairing_transport.rs`、`services/restore/mod.rs`、`services/fetchsymbols/mod.rs` 等专用入口仍需另行评估其原始 H2 初始化期限。
