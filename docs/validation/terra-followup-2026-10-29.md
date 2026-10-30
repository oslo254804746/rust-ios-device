# Terra 实现与总控验收（2026-10-29）

后续结果见[补全验收](completion-2026-10-29.md)：完整 archive/collect 已在明确的
512 MiB 预算内通过；取消路径的 RST 候选按项目原有 EOF/reset 契约校正为测试假设问题。
本文件保留当时的实测及未验收状态，不回写旧证据。

按用户要求派发 GPT-6.1 Terra，reasoning effort 为 xhigh（极高）。
两名代理分别实现 OS trace 和 debug，另加一名专项审查并发收尾候选；
主代理负责范围审核、串行 Cargo、独立主机验证及真机回归。
直接在本地工作区继续，没有提交或 push；既有未提交修复保留。

证据轮次 `20261029-terra-followup`，归档位于仓库外的
`C:\Users\wangbaofeng\ios-device-tests\20261029-terra-followup`。
分支 `codex/feature/windows-return-20261027`，HEAD
`ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`，受测的是有改动的工作区。
最终二进制 SHA256：
`3af180da0026844e41f7c3fe7ce0d3360392af6762dc629f91817223e8577c1a`。
`bin/verified.json` 保存各产品源码哈希并确认验证期间源码未变化。

## 已实现

| 部分 | 结果 | 验证边界 |
| --- | --- | --- |
| OS trace 独立主机预算 | archive/collect 新增 `--max-bytes`，与设备 `--size-limit` 分开；不传新参数时保持旧行为 | 协议测试验证设备忽略提示仍被硬上限拒绝、完整小归档成功、溢出/截断/原子发布/暂存清理 |
| OS trace 限额诊断 | 结构化 transfer/chunk/file/extracted/entries 类别，分类为 ResourceLimit，保留原错误源链 | 自动诊断只输出固定类别及相关 ID，不输出路径、设备身份或 daemon 文本 |
| debug 前台会话 | 原 userspace 隧道持续持有，确认远端连接后提供单客户端 `127.0.0.1` 端口，结束或取消时清理 | 实际 loopback 字节往返、ACK 失败/截断、连接前及传输中取消测试通过；不重放业务数据 |
| debug 服务与脚本 | 先检查 RSD 服务，再查询 app；缺服务明确报告前置不足；移除写死的工作区声明 | 路径转义、控制字符拒绝、raw script 反引号编码、`SBFileSpec(path, False)` 和设备反斜杠路径拒绝 |

解压总量、单文件、条目数、路径/链接检查和暂存原子发布防护继续保留。
debug 由原来打印后退出改为保持前台运行，详见 [debug 使用说明](../debug.md)。
打印的 `process launch` 由用户在 LLDB 中执行；ios CLI 不执行它。

## 总控验证

- attempt017：469 项核心测试通过，采用 `--test-threads=1`。
- attempt018：108 项 CLI 测试通过，3 项按原规则忽略，采用 `--test-threads=1`。
- attempt019：构建成功，随后归档二进制；以上 Cargo 调用串行执行。
- attempt021：本机 LLDB 22.1.8 对合成 arm64-apple-ios18.0 Mach-O 对象完成
  target 创建和平台文件元数据设置，验证空格、中文、引号与反引号。
  Windows 的 fullpath 使用本地分隔符，比较时明确归一；未执行 process connect/attach/launch，
  未验证远端 launch 报文。运行依赖仅复制到本轮私有目录，安装目录未修改。
- attempt025/026：非法主机预算在设备连接前被拒绝，无输出文件。
- attempt027：受信任 D2 仍为 iPhone11,8、iOS 18.7.10 / 22H374。
- attempt028：当前 RSD 不提供 debugproxy；CLI 输出固定前置提示，stdout 无脚本，exit1。
- attempt029：旧 1 MiB 限额行为保留，安全诊断为 `os_trace_archive_transfer_limit`。
- attempt030/031：最近 60 秒、age1d、设备提示 1 MiB、独立主机上限 32 MiB、45 秒总期限；
  archive/collect 均自然返回 ResourceLimit，未超时或强杀，没有发布部分输出或残留暂存。
  **两项完整真机归档仍未验收，没有继续放宽预算。**

首轮宏属性编译错误和错误源链断言失败均保留；总控发现并交回 Terra 修正。
attempt006 的另一个既有 userspace EOF 测试在并发测试中失败，单独两次以及完整串行套件均通过。
专项审查只增强断言诊断，严格 EOF 和 2 秒期限不变；attempt036 再次失败并捕获 Windows 10054
ConnectionReset。随后补充 IPv6/TCP/目标端口/SYN 前置断言，attempt038–042 连续五轮
各 469 项默认并发测试通过。**RST 根因尚未确定，保留待诊断候选，不能据此宣称全部并发稳定。**
未放宽断言、添加 sleep 或把 reset 当作 EOF；也未凭猜测添加无法保留真实清理错误的 Drop guard。
补充测试源码与受测 CLI 的产品代码分开记录；该 cfg(test) 诊断不改变产品二进制。
本机 LLDB 最初因缺 DLL 启动失败、初始路径比较未考虑本地分隔符的断言失败也保留，
不使用 exit0 代替语义断言。

脚本审查参考 LLVM 官方
[命令预处理](https://github.com/llvm/llvm-project/blob/main/lldb/source/Interpreter/CommandInterpreter.cpp)、
[参数解析](https://github.com/llvm/llvm-project/blob/main/lldb/source/Utility/Args.cpp)及
[SBFileSpec](https://github.com/llvm/llvm-project/blob/main/lldb/source/API/SBFileSpec.cpp)，
再以实际主机 LLDB 验证相应子范围；这些依据不构成 iOS 调试会话通过。

## 还需要补齐的条件

1. 完整 OS trace 归档：设备实际返回超过本轮 32 MiB 硬上限，需要更小的受控日志素材，
   或另行明确的足够输出预算；不能移除上限或接受截断 tar。
2. 真机 debug：需要提供 debugproxy 的设备、匹配 app/符号及调试执行范围。
   本轮只验收主机协议和当前设备的缺前置路径。
3. WebInspector：当前设备未启用且没有受控页面。
   核查未发现 Rust 收到禁用通知后静默丢失的证据；上游通过独立 notification-proxy 订阅，
   该通知观测能力仍需正向通知及生命周期素材，不能将普通 EOF/超时误判为 disabled。
4. 完整壁纸、真实 HCI、真实手表事件：沿用[前轮逐项结论](cli-diagnostics-2026-10-29.md)，
   分别属于当前端点限制或缺正向素材，本轮未改设置或制造事件。
5. F02/F04/F06 的下载、可信 CDP Input、完整 restore update 等仍为先前明确搁置范围，
   本轮没有将一般补全请求扩展为这些执行器开发或危险实测。
6. userspace 并发停止的 Windows RST 候选：需进一步确定客户端关闭时序与业务收尾契约，
   当前只确认真实 reset，没有确认泄漏或定位能安全修复的代码原因；详细审查在外部归档
   `reviews/lifecycle-review.md`，原失败及后续成功均保留。

原 companion/symbols 等改动经哈希核对保留，回归包含对应核心协议测试。
所有本轮服务均退出，既有 2026-10-20 的 ios PID36976 未触碰。
没有新增信任、配对、DDI、系统路由、kernel tunnel、HID、设置/剪贴板或个人 app 操作。
设备日志、图像、进程/app 数据均留在 private，不入 Git。
