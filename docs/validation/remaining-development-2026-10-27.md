# 剩余开发增量 — 2026-10-27

基线 `a7ca50e22770eec7d52f27ebe85a925427cc0859`，分支
`codex/review-fixes-20261010`，同一checkout，仅本地提交。
本轮由主控整合，三个 gpt-6.1-terra/xhigh 子代理分别开发 L01、F02、F06，
随后F06代理接续F04 world/console。子代理不执行Cargo/rustc/build/fmt/commit；
全部源码冻结后由主控串行验证。没有真实设备发现、配对、安装、启动、恢复、TSS或系统网络操作。

当前有效30项为 **27项开发交付、3项仍未完成：F02/F04/F06**。
L01按有限生产profile开发口径收口；其raw兼容入口、OS syscall物理不可抢占与外部NOT_RUN
明确保留，不扩大为全API/全进程证明。F01/F05承接前一批开发交付，原L02/Q04口径保持。

## 实现与原范围

| 项目 | 实际开发 | 边界 |
| --- | --- | --- |
| L01 | owned ZIP不再调用全量展开loader；有界ZIP32目录快照准入、分块CRC预检、64KiB增量Stored/Deflated解码与发送共享原clock | 同步OS文件调用不能物理中断；仅受控ZIP32/Stored/Deflated profile，borrowed兼容API原样保留；文件payload被外部原位修改可在第二遍CRC失败，不能声明文件快照或无远端效果。 |
| F02 | 同一个TestWorkflowEngine::run在真实localhost readiness/RSD/AppService/DTX peers上执行canonical成功、shim成功和launch后授权失败；通过testcase落到JUnit/sidecar；PID kill先于DTX关闭 | 注入是显式task-local测试边界和test-tunnel+mdns构造；不匹配目标直接拒绝，不能回退真实发现。人工loopback handle没有native任务，不证明原native退休；不是binary子进程或安装/日志整链。 |
| F04 | normal/user/internal按context ID与世代保存；同frame多world求值/退役，normal索引专用于导航；对象console可证明子集 | 原生四字段与frame securityOrigin形成CDP world描述；objectId里的injectedScriptId需一致指向活跃未复用realm，加有限设备WallTime，才产生Runtime.consoleAPICalled。primitive-only/冲突/缺时间/退役仍保留原事件。可信Input/下载仍OPEN。 |
| F06 | 新实验restored loop实际派发同步SystemImageData到已有ASR，继承control总clock/cancel，传输后继续读控制消息；保留部分执行报告 | AsyncDataRequest明确拒绝；ASR收据及Status0不能把不完整Update宣布完成，CLI force/tss维持关闭。完整模式/nonce个性化/USB重识别/FDR/异步dispatch继续缺开发。 |

ZIP受控profile：archive ≤ u32::MAX、16384 entries、目录声明 ≤16MiB、
metadata parser累计read ≤32MiB、单条展开 ≤1GiB、总展开 ≤8GiB，输入/输出块各64KiB。
只识别comment结束于EOF的outer EOCD，内嵌ZIP与数据中的PK0506不当作外层footer。
中央目录实际交给zip库的是已准入快照，之后修改原文件count不能改变parser分配依据。
每条按声明尺寸、真实CRC和compressed range检查，初验前不发送InitTransfer。
它是有界本地准备/流式传输，不是整进程RSS证明或文件系统快照服务。

## 协议与复核

F04 wire使用固定[Runtime.json](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Runtime.json)的`contextId`。
主控一度从C++参数名误判为executionContextId，子代理核对JSON后阻止该改动；
当前fixture额外拒绝错误executionContextId字段。world/对象关联来自固定
[PageRuntimeAgent](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/WebCore/inspector/agents/page/PageRuntimeAgent.cpp)、
[InjectedScriptSource::_bind](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/InjectedScriptSource.js)、
[ConsoleMessage::addToFrontend](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/ConsoleMessage.cpp)。
Console type=log须按原level区分log/info/warning/error/debug；不从URL猜world。

固定[Automation.json](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/WebKit/UIProcess/Automation/Automation.json)
提供browsing-context原生mouse/key，但当前Inspector page没有到该automation handle的已验证身份绑定；
没有下载控制方法，不能用其存在宣称当前CDP可信Input或download已实现。
F06同步SystemImageData/DataPort按固定[pymobiledevice3 restore.py](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/restore.py)实现，
没有将async任务改作同步成功。

整合修正还包括：ZIP不扫描全部payload的伪footer；目录元数据准入后使用稳定快照；
workflow测试采用JoinSet，场景错误/panic/独立安全期限到达均abort并实际join全部peer；
成功场景必须包含1个真实通过case，核对JUnit与sidecar，不用空plan终态代替结果。

## 验证和提交

最终受测代码提交 `8ea9af31613154c61d4efc3f825e5672362f707d`；此后仅文档/能力登记更新。

冻结源码的门禁见下表；早期失败与修正保留在表后。

| 已完成命令 | 实际结果 |
| --- | --- |
| Core ZIP定向 | 31/0/0 ignored；0.30s，`.git/terra-dev-zip-final.log` |
| Core restored定向 | 13/0/0 ignored；0.06s，`.git/terra-dev-restored-final.log` |
| Core+CLI all-features/all-targets strict Clippy | PASS，43.87s，`.git/terra-dev-clippy-verified.log` |
| 同一engine现代RSD/AppService/DTX场景及注入目标隔离 | 2/0/0 ignored；0.27s，`.git/terra-dev-modern.log`，包含canonical成功/shim成功/授权失败及零真实发现 |
| 显式固定Puppeteer新增world场景 | 1/0/0 ignored；4.09s，`.git/terra-dev-world-client.log` |
| 显式固定客户端全组 | 16/0/0 ignored；39.13s，`.git/terra-dev-clients-all.log`；运行前--list确认16项 |
| Core lib all-features完整 | 2041/0/1原ignored；33.23s，`.git/terra-dev-core-full.log`，包含world20/CDP33、restore91、ZIP31 |
| CLI当前精确lib制品完整4线程复验 | 1986/0/22 ignored；66.90s，`.git/terra-dev-cli-verified.log` |
| 外部Rust workflow API | 1/0/0 ignored；`.git/terra-dev-workflow-api.log` |
| 显式真实FFmpeg媒体 | 3/0/0 ignored；0.20s，`.git/terra-dev-media.log` |
| 最小Core feature组合 | PASS，原13条dead_code警告；`.git/terra-dev-minimal.log` |
| CLI all-features build | PASS，55.87s；`.git/terra-dev-build.log` |
| fmt --check / git diff --check | PASS / PASS |
| 能力登记 / 策略测试 | PASS：119 records与实际CLI --help一致；策略测试6/0，`.git/terra-dev-capabilities.log` / `terra-dev-capability-tests.log` |


- ZIP初次编译因Cursor同时提供同步/异步read出现歧义；明确std::io::Read后，31项定向通过，`.git/terra-dev-zip-final.log`。后续整合源码再由完整批次覆盖。
- restored首轮13项为10通过/3失败：新拒绝路径fixture错误传入空RestoreOptions，尚未发送StartRestore就失败；两处通用source遍历跳过了stdio内部取消payload。改为有效Update options并直接遍历IO.get_ref核对实际OperationCancelled，保留原取消/连接失效/late-success断言，最终restored13项及Core全量均通过。
- world初轮20项为19通过/1失败：新tree fixture漏URL，严格valid_tree拒绝。补齐原生frame字段，未放宽validator或退役断言；最终完整Core2041批次覆盖通过。
- strict Clippy先拒绝中央错误归档后剩余的identity map_err，随后编译发现提取launch helper借用整个cleanup与既有device借用冲突；移除无效map并只传递独立controller/PID/提交状态字段，原流程/收尾不变，最终strict通过。
- 生产复核修复两份取消token：循环token现在也覆盖StartRestore和同步firmware写入；所有错误退出统一terminal并保留部分报告，ASR内部token取消的报告也记Cancelled。
- CLI首轮完整为1984通过/2失败/22 ignored，失败为未修改的两项snapshot_admission时序fixture。单线程同组复查11/1，随后同制品USB exact单项在原期限下1/0（1.10s）。没有修改L02源码/预算/断言，使用同一Cargo产出制品完整4线程复验1986/0/22 ignored通过；此处不把早期失败隐藏或解释成确定已定位的产品回归。
- ASR失败时没有完整receipt，部分报告字节数只是已确认完成发送的下界，0不证明没有任何部分写入；不声明设备回滚或闪存进度。


## 本地提交与精确制品

| 提交 | 交付 |
| --- | --- |
| `98768b4` | 受控ZIP32目录快照、64KiB双遍CRC/增量解码及31项ZIP门禁 |
| `a83cf84` | 同一workflow现代RSD/AppService/DTX/testcase/报告fixture及安全收尾 |
| `6b2c374` | isolated worlds、逐ID退役与可证object console、固定客户端16 |
| `8ea9af3` | restored同步SystemImageData dispatch、独立取消与统一terminal/partial report |

| 当前制品 | SHA256 |
| --- | --- |
| `target/debug/deps/ios_cli-e7a77098f7474c2d.exe` | `740C21816814CE03407F948A7AD6EAEC65B0AC1AA9F179D6BED305FDC9BA3AB9` |
| `target/debug/deps/ios_core-f104c74c4ac12fce.exe` | `9EB302F7C17E32AD332AEE344D29F1DEC2C5A7C70F46A2CA47724EBBD02034A9` |
| `target/debug/ios.exe` | `2F18EA77F3367EB48C443C565448B8B0262D5B6DDCA7C4067CD97A13BA8811B2` |

复现命令（Cargo全部由主控串行，显式制品先按Cargo输出确认）：

```powershell
cargo test -p ios-core --lib --all-features --locked zipconduit:: -- --test-threads=4
cargo test -p ios-core --lib --all-features --locked services::restore::restored::tests:: -- --test-threads=4
cargo test -p ios-cli --lib --all-features --locked cmd::test_workflow::modern_fixture_tests:: -- --test-threads=1
cargo test -p ios-core --lib --all-features --locked -- --test-threads=4
& target/debug/deps/ios_cli-e7a77098f7474c2d.exe --test-threads=4
& target/debug/deps/ios_cli-e7a77098f7474c2d.exe cmd::webinspector::bridge_tests::client_tests:: --ignored --test-threads=1
& target/debug/deps/ios_cli-e7a77098f7474c2d.exe cmd::viewer:: --ignored --test-threads=1
cargo test -p ios-cli --all-features --locked --test workflow_api -- --test-threads=1
cargo clippy -p ios-core -p ios-cli --all-features --all-targets --locked -- -D warnings
cargo check -p ios-core --no-default-features --features testmanager,heartbeat,restore,amfi,power_assertion,webinspector --locked
cargo build -p ios-cli --all-features --locked
cargo fmt --all -- --check
```

CLI的22 ignored中16客户端和3媒体已显式执行；Doctor child、S02浏览器和L04测量入口不计PASS。
Core原1 ignored也不计PASS。无0匹配门禁；未放宽期限、删除或ignore旧断言。

## 外部未运行

真实iOS/TSS/恢复、物理输入/剪贴板/媒体、macOS/Linux和独立桌面客户端按原报告保留NOT_RUN。
用户没有iPhone14,2/d63ap iOS17.5.1/21F90离线IPSW；不重复询问、不以缺文件掩盖完整执行器缺口。
原L02收口和Q04全部排除保持，主项分母30。

## 2026-11-07 F02 / F04 / F06 最终代码收口与真机/全量验证

分支 `codex/feature/windows-return-20261027` 完成剩余三项（`F04`、`F02`、`F06`）代码收口：

1. **F04（`c71fd5cdda785d467a55a0baa21f45f88424beb7`）**：
   - `crates/ios-core/src/services/webinspector/cdp.rs` 实现 primitive-only `console.log`/`info`/`warn`/`error`/`debug`（无 `objectId` 参数时）向当前活动主框架默认 `normal` `contextId` 合成 `Runtime.consoleAPICalled`，同时保留含 `objectId` 时的 `isolated world` 与 `injectedScriptId` 世代/退役校验；
   - 补齐受控 CDP `Input.dispatchMouseEvent`、`Input.dispatchKeyEvent`、`Input.insertText`、`Input.dispatchTouchEvent`、`Page.setDownloadBehavior`、`Browser.setDownloadBehavior` 以及 `Performance.enable`/`Performance.getMetrics`；
   - 固定客户端 16 项全过，真机 `00008020-0004553E02F2002E`（`D2`，iPhone11,8，iOS 18.7.10）`WEB-001..WEB-023` 全量 23/23 `PASS`（`WEB-019`、`WEB-020` 消除 `KNOWN_GAP`）。
2. **F02（`9caed46d1090a69f98f697898760cded78e20031`）**：
   - `crates/ios-core/src/services/testmanager/workflow.rs` 提供可复用 `execute_reusable_test_workflow_plan` 编排入口与 `reconcile_unconfirmed_runner_launch` 未确认 PID 进程查回/清理；
   - `crates/ios-cli/src/cmd/test_workflow/modern_fixture_tests.rs` 新增串联 ZIPConduit/InstallationProxy 安装 + 现代 RSD/AppService/DTX 测试执行与重跑 + Syslog/OSTrace 采集 + JUnit/sidecar 报告 + 卸载 + 原生 `NativeScope` 隧道任务退休的端到端回归测试（`test_workflow` 25 项、`runtest` 14 项、`workflow_api` 2 项通过）。
3. **F06（`a2a805a8dd9cdf26e65e8de86b5bee7cfeb741bb`）**：
   - `crates/ios-core/src/services/restore/{img4.rs, recovery.rs, restored.rs}` 对齐 `pymobiledevice3/restore` 补齐 ASN.1 DER `IM4P`/`IM4M`/`IM4R` 组装、Recovery/Ramdisk 重枚举衔接、31 类 `restored` 同步/异步 `DataRequest` 状态机与 ASR 流转；
   - `crates/ios-cli/src/cmd/restore.rs` 提供受控 `--confirm-simulation` 校验门禁的本地模拟恢复 CLI，真机真实刷机保持安全拒绝（`services::restore` 95 项、`cmd::restore` 18 项通过）。
