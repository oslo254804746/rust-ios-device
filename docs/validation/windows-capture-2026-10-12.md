# Q02 通用 Windows capture/agent · 2026-10-12

承接 db5487f 的原 FUP 核查，在现有本地 checkout 修复通用捕获；没有新建 checkout、
派发子 agent、推送或操作真实设备。Rust 与 CLI 二进制未修改、未重编译。

## 改动

`diagnostics.capture` 与 `Session.agent` 现使用 Windows 10+ 原 Job/process owner。
共享 `regression_windows_process.py` 从 watch 提取原 Job/handle 与原子启动机制；
普通捕获直接创建隐藏 child，`JOB_LIST` 在执行前准入，显式继承 stdout/stderr
句柄、NUL stdin。两路原始 bytes 不经过 controller，不插入元数据或解码重编码。
报告新增 `windows_owner_sha256`，明确记录共享实现来源。

退出码保留原 child DWORD，包括 259；Windows 强制终止沿用 Popen 的 1。
deadline 包含启动时间，原累计输出上限、raw 与独立 masked 扫描保持。
超时、stop、overflow、read/close failure 和 reader 启动异常均回收本轮 owner；
leader 成功但后代关闭管道仍存活也检查/清理原 Job。agent 在 leader 已退出、
后代仍持有管道时可立即停止，保留 leader 原退出码，不等待业务总期限。
只操作持有的 Job/handle，无按名称/PID 重新发现终止；Job 空、原 process 完成和
原 reader joins 才能报告 host cleanup。强制回收不是 native/device ACK。

API 依据：[JOB_LIST/HANDLE_LIST](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)、
[CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)。

## 验证

- 新 Windows capture 门禁最终 10 项通过：非空二进制 stdout/stderr、Unicode argv/env、
  原 cwd/退出码、第一条业务操作检查本轮确切 Job、closed-pipe child/grandchild、
  unrelated sentinel、超时、overflow、读/close 失败、启动准入失败、原 handle 关闭、
  原 readers join、Session agent 原秘密扫描及 leader 退出后的停止。
- 受影响组合 104 方法：99 PASS / 5 平台跳过，包含 watch 实际 TLS startup
  Ctrl-C/strict EOF、AFC/XCTest/crash/WDA/RSD/InstallCoordination localhost 实际 CLI。
- 随后仅修改 stop 条件并新增对应测试，重跑直接受影响的 capture/agent/watch：
  49 方法，45 PASS / 4 平台跳过；其余已通过且无 stop 使用的门禁按影响复用。
- 实际主机 runner：5 PASS / 0 FAIL / 22 NOT_RUN，`real_device_executed=false`。
  原 CLI SHA256：`b720cfce86ea2c61575480c73979f57a72c6ff8f0c89c304ea486870b049b62f`。
  语法、diff、source/log hashes 和提交关联见外部 summary；不称全量 Rust/SDK 重跑。

失败日志保留：最初 Windows 新门禁 7 PASS / 1 ERROR，测试误把 Session 当作
context manager；下一组合 71 PASS / 1 ERROR / 5 skip 同因。中间修正脚本遇到
Windows 默认 GBK 读取 UTF-8 文件失败、未修改源码，后用显式 UTF-8 修正。
最终测试没有删减原断言；首次失败未进入该 agent 用例，不伪称当时已经验过。

## 剩余边界与交接

本项完成通用 process-tree 接线，不单凭工具子项关闭 Q02。原 FUP 历史报告/Windows
证据 root 不可用，Windows ACL/第三方 adapter 与所有者存储边界单列；真实设备、
其它平台与最低 Python 版本仍未执行。完整 rich clipboard、paired-identity CLI/mDNS、
kernel Display/HID、其它 RSD 模式与现代工作流仍保持原 NOT_RUN/开发核查范围。
当前账本保持 22 交付、7 缺开发、L02 待完整收口；有效30，Q04及全部子项排除。

下一步：按原 FUP 条款继续 Q02 收口，区分代码缺口与仅运行条件；然后推进 L02
跨子系统原 error payload 字节归因。每个可验收项验证后立即本地提交。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\capture`。
入口 `windows-capture-summary.json` 和 `round-handoff.json`；源码归档、日志及旧失败
制品均保留。提交后的未提交改动与下一步由 round-handoff 记录。
