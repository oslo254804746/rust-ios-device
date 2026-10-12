# Q02 Windows watch工具交付 · 2026-10-12

上一组协议observer/XCTest/WDA修复已本地提交`94574d6`。本项按用户新要求：
验证完成即本地提交，写明改动与限制，再继续原范围下一项；不推送、不操作真实设备。

## 原进程与信号所有权

`OwnedWatch`现支持Windows 10+，使用隐藏的独立console和私有Job Object。
子进程通过`PROC_THREAD_ATTRIBUTE_JOB_LIST`在CreateProcess时原子进入Job，
不先执行再补准入。标准输出/错误和NUL stdin使用显式handle list，控制pipe不传给CLI。
控制器启用子进程Ctrl-C后才为自身设置忽略属性；每次发信号前核对console中仅有
控制器和本Job的进程、调用者PID不在其中。没有AttachConsole/FreeConsole或进程名kill。

控制器原process handle、Job handle和输出reader均保留到结束。正常退出核验Job
active processes为零并join两个原reader；残留child即使关闭pipes，仍触发Job失败清理。
输出有总字节预算，截断NDJSON拒绝；finish可重复调用，不重新发送或重新创建owner。
只有原CLI的stopped/native_reclaimed、正常退出、host Job空和reader joins同时满足
才能返回native receipt。强制TerminateJobObject、仅退出/Drop/EOF均不能提供该receipt。

`rich-clipboard-watch`不再在Windows入口直接NOT_RUN。生产adapter仍先验证可恢复的
hash-bound baseline，再启动watch/变更合成内容；baseline失败只执行read-only get，
不启动watch或写入剪贴板。完整设备文本/图像/去重/恢复流程本机仍NOT_RUN。

协议依据：[Ctrl-C发送范围](https://learn.microsoft.com/en-us/windows/console/generateconsolectrlevent)、
[忽略属性与子进程继承](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler)、
[JOB_LIST与HANDLE_LIST](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)、
[Job子进程与释放](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)。
不设置breakaway；旧Windows版本不扩大支持声明。

## 验收

| 检查 | 结果与边界 |
| --- | --- |
| Windows新门禁 | 9通过：7个原console/Job/reader进程用例、1个预算准入、1个实际CLI |
| 进程用例 | 原SIGINT、无关本轮sentinel继续运行、忽略信号、leader退出但child存活、无receipt、非法可执行文件、输出overflow、末尾NDJSON截断、重复finish |
| 实际CLI | `pasteboard watch --reconnect`在真实localhost TLS Lockdown/CoreDeviceProxy StartService等待中收到Ctrl-C，原生产stopped/native_reclaimed=true、exit0；独立原peer严格EOF/worker join |
| 受影响合同 | 49方法，45通过/4平台跳过；原native cleanup receipt与observer负合同不削弱 |
| Rust/二进制 | Rust未变，不重编译；使用已验证CLI SHA256 `b720cfce86ea2c61575480c73979f57a72c6ff8f0c89c304ea486870b049b62f`，提交blob等价核验另存 |
| 最终目录/语法/diff | 结果及精确hash见外部summary |

JSON children仅验证host supervisor。实际CLI gate另使用隔离现有信任目录与真实TLS
wire；停在服务启动前，不创建实际TUN、不执行剪贴板变更。它不能替代完整通知/媒体/
剪贴板设备流程。WDA5项与上述watch取消属于不同原门禁，不合并为一个物理工作流PASS。

失败保留：初始7项中6通过/1失败；实际CLI fixture错误使用不存在的root `--json`
参数，未进入TLS IO。focused诊断失败保留；改成生产`--error-format json`后实际CLI
单独通过，最终9项完整重跑通过。初始受影响合同44通过/1失败/4跳过，旧Windows
NOT_RUN断言改为生产adapter准入和失败baseline不变更合同后，最终45通过/4跳过。

日志/来源/commit关联：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12df9-97f9-7771-bc97-dc1b21bc779e\repair\watch`，
入口`windows-watch-validation-summary.json`。原失败日志、旧WDA与integration制品保留。

下一步按Q02原定义继续FUP-01–FUP-04的完整交付边界复核；不因9个新方法直接关闭
整个Q02。L02全局原错误payload归因及其他原缺口继续；有效30与Q04全部排除保持。
