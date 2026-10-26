# runtest 现代启动分支修复 — 2026-10-26

起点`7fb6e19`，同一分支/checkout。本次只修一个明确接线错误，不新增工作流框架。

`runtest`的首次执行、失败重跑、仅启动，以及保留的Rust便捷入口，原来均向现有启动器传`prefer_coredevice_runner=false`，iOS17+因此继续走Instruments。现在前三者调用已有`start_test_plan_session_owned_until`，便捷入口启用同一版本选择；实际17+使用现有CoreDevice AppService，旧系统仍由原版本判断选择Instruments。WDA专用入口保持其原协议路线。

验证：
- `cargo test -p ios-cli --all-features cmd::runtest::`：11 PASS，0 FAIL。
- 同一测试二进制`target/debug/deps/ios-57c13d5e477e2e12.exe cmd::runxctest::`：10 PASS，0 FAIL；仅换行格式整理后复用该二进制，无逻辑变化。
- `cargo clippy -p ios-cli --all-features --all-targets -- -D warnings`：PASS。
- `cargo fmt --all -- --check`、`git diff --check`：PASS。

这里只证明编译、既有定向回归及生产接线修正，不把参数/transport-plan测试计为现代完整端到端验收。真实设备安装/启动和现代全流程仍NOT_RUN，F02其它原缺口继续；L02/Q04范围不变。
