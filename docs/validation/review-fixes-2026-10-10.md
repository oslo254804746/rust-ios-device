# 交付审查修复工作包

开始：2026-10-10；续行/集中验收：2026-10-11（Asia/Shanghai）。起点 `af20f6aa3752d2511b763b14239cda78a1db9bef`，
分支 `codex/review-fixes-20261010`。原交付基线 `993a366` 与迁移起点 `660bd46` 保留。
实现工作包已本地提交：`88d8f707f6f23bc72b9c0aa80230ad0739752f03`。
本页后续证据维护不改变该受测产品源码；没有推送、发布或部署。

审查拒绝了 `af20f6a` 的“30/30”结论；历史报告中的测试数量不能作为产品流程完成证据。
原19项作为账本承接，L03增加1项；本轮A03完整开发与绑定gate交付再增加1项。
当前21项可承接、8项仍缺开发、L02待完整核查，不能宣称30/30。
Q04及其所有子项继续排除。

## Restore（R04/R05，F06）

正常模式 `com.apple.RestoreRemoteServices.restoreserviced` 的 RemoteXPC 控制协议
与USB恢复模式 `com.apple.mobile.restored` 的plist协议分别实现，不再混用。
旧循环失败为Err，原原因和部分报告通过 `RestoreExecutionError` 保留；
取消唤醒idle receive，控制连接进入terminal。codec准备量与实际写入量分开。
数据请求或没有StartRestore/传输证据的成功状态不能生成Completed。

新增 `RestoredClient` 对已经连接的restored transport执行QueryType、HardwareInfo/ECID
核对，检查明确协议版本和Update options后发送StartRestore，实际写入同步
FirmwareUpdaterData的header、chunks、DataDone。所有步骤共用绝对期限与
OperationCancel，错误后禁止复用；只计成功flush的payload。专用peer检查完整wire、
实际EOF、错误ECID和取消。这是传输子项，不是完整恢复执行器。

依据为只读pymobiledevice3固定版本 `03ba2270` 的 `restore/restored_client.py` 和
`restore/restore.py`；没有复制参考源码。仍缺recovery ramdisk/USB recovery transport、
IMG4个性化和Apple票据签名验证、ASR系统映像校验/传输、FDR、DataPort/AsyncDataRequest
owner、物理USB重举与完整机型/固件计划。CLI真执行在连接设备前拒绝，F06仍缺开发。
不会用--force、状态枚举或局部真实传输关闭F06。

`update --dry-run` 是只读设备查询与档案检查，会查询身份和nonce，不是离线演练。
它读取selected Manifest全部组件到EOF，检查ZIP CRC、大小、重复路径、缺失文件、
总32GiB输入预算，记录SHA256；未解释全部Apple Manifest Digest语义。
`--tss` 必须是带原Request和Response的plist envelope，核对ECID/板号/芯片/AP与SEP
nonce和非空ApImg4Ticket；原始票据不能证明关联而拒绝。输出明确
`apple_ticket_signature_verified=false`、`execution_supported=false`。

每updater建连和请求共用时钟；CLI另限制整个连接/查询/聚合总期限。聚合失败
输出部分证据后返回非零。支持版本的实体trace仍缺，不能据参考名称扩大版本支持。

## Manager（L02）

全局clear/POST epoch取消旧ticket并重置desired intent；普通扫描保留持久Stop/Manual。
这是原容量/清理合同，不能把全局clear声明成保留Manual/Stopped。新intent版本和epoch
原子更新；终态关闭单调拒绝。发布目录、身份、凭据buffer
容量共用64MiB准入，撤销/替换/拒绝后由原owner释放。不透明原生错误对象、native runtime
与跨子系统所有历史元数据尚未全量归因；刷新/复用/重启全合同继续，L02不关闭。

## Windows原失败诊断

本机原生loopback拒连实测约2.05–2.15秒才产生ConnectionRefused；原fixture的
1–2秒外层时钟先到，public retry另有五次实际连接及500ms间隔。拒连fixture使用
平台相关观察预算，保留原ConnectionRefused/source断言，独立timeout fixture继续。
没有把拒连换成合成error。

Backup2暂存payload完成Tokio write_all后flush再进入下一网络等待，防止末次blocking
write与取消后删除暂存文件竞态。Windows目录枚举在write句柄未关闭时实测缓存长度0，
fresh read句柄观察实际长度7；fixture改为核对实际partial字节。旧目标/EOF/清理断言保留。这不证明
stalled filesystem的worker可物理取消，L01该范围继续。

## 验证记录

Rust/maturin由主agent串行构建，检查其它编译进程，不终止他人任务。开发期针对
复制后的冻结源码快照；最终验收固定文件hash与受测提交。初始失败日志保留，
修正后按影响范围重跑，不转换成NOT_RUN。
真实设备恢复、配对、安装、擦除及状态变更均未执行；真机由用户后续统一安排。
POSIX/glibc及其它平台未运行范围准确保留。

## 已完成的修复与范围

| Review | 本轮具体结果 | 仍缺的原范围 |
| --- | --- | --- |
| R01/R02 | Web/VNC持有UDID/监听端口范围Principal；过期/撤销贯通等待。策略模式VNC须owner token+密码，DES已知向量与真实TCP握手通过。 | 标准VNC客户端/设备端到端未运行；VNC文件策略变更须重启，Web有授权reload入口。 |
| R03 | 移除固定页面、1x1图、常量指标、空输入/Fetch成功。Page/PNG/有限Fetch从真实Inspector回复与事件关联。修复新定时器中断partial native read的回归。 | 可信Input、Memory/Performance、完整Fetch/download和各域pinned-client流程缺实现/证据，F04不关闭。 |
| R04/R05 | 正常控制服务不能假执行restored流程；失败Err、部分报告/原原因、取消/terminal明确。restored真实StartRestore/firmware子项与档案检查已实现。CLI真Update连接前Unsupported。 | 完整机型/固件路径、USB/recovery/IMG4/ASR/FDR和签名/版本证据仍缺，F06不关闭。 |
| R06/R13/R14 | 新旧apps/AFC/logs共用checked owner；期限/输出/并发/取消/close、真实service TLS和log partial通过安装后及C ABI gate。aclose公开入口通过。local pair按已有A02为invalid_input，保留parse cause。 | 真机与其它平台验收保留；legacy通用Lockdown与文件loader的阻塞范围不冒充checked profile。 |
| R07 | 真实PNG像素/Web/VNC共享层，设备HID及文本pasteboard有ack；clipboard GET返回内容，sharing开关生效。原native registry贯通连接/回收，服务缓存保留连接。 | HEVC、方向、图像clipboard、重连完整流程和全进程64MiB预算仍缺，F05不关闭。 |
| R08 | 根--udid、严格manifest、观察/授权分开、真实路径/错误/hash/本轮清理；9个文件/Windows子case经实际CLI+TLS/AFC peer验证。 | 6个需要wire/restoration observer的case明确未实现，不以info/list替代；真实设备22行NOT_RUN，Q02不关闭。 |
| R09/R10 | 包内真实ID、安装前不存在状态与本轮追踪；finally收集测试/session/log/app/report错误。连接stop监听、超时abort原handle并await join、限制日志输出。 | 各完整安装/XCTest阶段peer与重跑历史流程仍需覆盖，F02不关闭。 |
| R11 | Windows/macOS有真实平台枚举；目标IPv6TCP route实际观察，未选择/未探测为Unknown；假dmg目录拒绝，继续后续目录。 | remoted共存、目标DDI签名/manifest/实际可用性仍未完整实现/证实，F01不关闭。 |
| R12 | 删除普通TCP/Vec假负载；manager原native ICMPv6、生产AFC与磁盘、Syslog生产parser实际时窗/资源样本。汇总保留physical/DTX NOT_RUN与所有fixture helper hashes。 | 前后测量结果继续核查；DTX、Linux资源归因/其它平台/物理负载独立保留，L04不据三个函数关闭。 |
| R15 | Viewer登记为experimental partial，source resolver支持目录mod.rs，实际CLI inventory gate通过。 | 不升级设备成熟度。 |

## 实际命令与结果

Windows x86_64/MSVC，rustc/cargo1.90.0，uv CPython3.13.5；Debug构建。
日志保存在本轮Codex `repair` 工作包；初始FAIL及所有后续修正重跑均保留。

| 命令 | 实际结果 |
| --- | --- |
| cargo fmt --all -- --check | PASS |
| cargo clippy --workspace --all-targets --all-features --locked -- -D warnings（PYO3_PYTHON已设置） | PASS，无warnings |
| cargo test -p ios-core --all-features --locked --lib -- --test-threads=1 | 1622 PASS、0 FAIL、1 ignored；原五项Windows FAIL均通过 |
| cargo test -p ios-cli --all-features --locked --bin ios -- --test-threads=1 | 1273 PASS、0 FAIL、5 ignored |
| 同CLI的bridge_tests::client_tests:: --ignored --nocapture | 3 PASS，真实Puppeteer25.12.0/Playwright1.63.0；Playwright初始化拒绝仍是工作流缺口 |
| cargo test -p ios-ffi --locked --lib -- --test-threads=1 | 38 PASS |
| cargo test -p ios-py --locked --lib -- --test-threads=1 | 22 PASS；打包extension-module未用于host test |
| 实际C DLL scripts/sdk-device-services.py --ffi-library target/debug/ios_ffi.dll | 30场景PASS，含新旧exports、TLS、四阶段total/cancel、共享query冲突、close、partial/secondary与原peer/OS-thread回收 |
| 新wheel安装后 python -I scripts/sdk-device-services.py | 30场景PASS，含Future.cancel原native join、partial、公开Device.aclose与重复close |
| 安装后 python -I scripts/python-device-query.py | PASS；Windows Unix socket子项仍NOT_RUN |
| Python unittest discover -s scripts | 89总数，65执行PASS、24skip |
| unittest test_regression_adapters.py | 8 PASS，实际CLI参数与9个合成文件/Windows矩阵子case；不计真机PASS |
| scripts/capabilities.py --binary target/debug/ios.exe | PASS，119 records |

L04实际运行同一fixture及 `workloads.rs` helper、同一driver、Rust1.90.0/Debug与Windows主机：
基线 `af20f6a` 仅叠加测量fixture，修复版 `88d8f70`。输入为1/4/16、每scale一次、
30s下限/至少16轮、250ms pause、256KiB payload、500ms Syslog。两版实际均完成16轮，
原生门禁、收尾证据及driver产物/输入/源码hash比较通过。修复版longrun实际151712ms；
该值包含每轮真实负载，不能把30s下限当作总运行耗时。

16设备分组的272次native ICMP负载样本，吞吐p50基线约9.926MB/s、修复约9.945MB/s；
每次负载耗时p95基线47.027ms、修复37.891ms。只是本机Debug观察，未建立跨平台
性能承诺或普适改进结论。详细前后分布、AFC、Syslog、原owner/Tokio恢复见
本轮 `load-comparison.json` 及before/after的fixture/summary/run/log。

第一轮基线原生测试本身通过，但pretty JSON汇总超过4MiB上限；保存该driver FAIL。
改紧凑JSON保持原字节上限和完整样本，使用同一修正后的driver重跑两版并通过。
新增helper文件也纳入fixture manifest，不能只比较父rs文件。
Windows RSS/FD及Linux/glibc原探针本轮未运行；TCP转发/DTX负载adapter与物理工作流
仍有缺口，不把native ICMP或Syslog样本替换这些NOT_RUN。L04仍缺完整原交付。

Python native host初次执行出现DLL_NOT_FOUND，随后出现encodings未定位。保留失败：
使用同一uv解释器DLL、PYTHONHOME指向其真实stdlib后22项实际运行通过。这是host
embedding运行环境设置；已安装wheel在真实解释器中独立通过，未用extension-module
掩盖host链接问题。
