# 交付审查修复工作包

日期：2026-10-10（Asia/Shanghai）。起点 `af20f6aa3752d2511b763b14239cda78a1db9bef`，
分支 `codex/review-fixes-20261010`。原交付基线 `993a366` 与迁移起点 `660bd46` 保留。
最终命令、受测文件清单和结果在集中验收后补齐，当前条目为实施记录。

审查拒绝了 `af20f6a` 的“30/30”结论；历史报告中的测试数量不能作为产品流程完成证据。
原19项作为账本承接，L03增加1项；其它10项须按原路线图逐项核查。
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

## 验证记录（进行中）

Rust/maturin由主agent串行构建，检查其它编译进程，不终止他人任务。开发期针对
复制后的冻结源码快照；最终验收固定文件hash与受测提交。初始失败日志保留，
修正后按影响范围重跑，不转换成NOT_RUN。
真实设备恢复、配对、安装、擦除及状态变更均未执行；真机由用户后续统一安排。
POSIX/glibc及其它平台未运行范围准确保留。
