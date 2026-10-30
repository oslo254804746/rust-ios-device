# 补全验收与测试契约校正（2026-10-29）

本轮按用户“继续补齐，不做过度防御或凭空问题”的要求，沿用 GPT-6.1 Terra / xhigh
分别复核归档参数和收尾测试，主代理审核、串行构建测试并执行只读真机验证。
仅一处产品逻辑精简：删除 CLI 对 `u64::MAX` 的任意拒绝；没有增加关闭 guard、重试、
错误账本或协议 fallback。零值校验、解析器的整数范围检查和现有累加溢出检查保留。

分支 `codex/feature/windows-return-20261027`，HEAD
`ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`，受测为保留原有改动的未提交工作区。
二进制 SHA256：`f1eefb38e8b855368620b0e97454cc94688717fddb46826c224aac83529eac69`。
仓库外证据轮次为 `20261029-completion`，位置：
`C:\Users\wangbaofeng\ios-device-tests\20261029-completion`。
`bin/verified.json` 绑定源码哈希与程序，`private/attempts/` 保存精确 argv、原始输出和退出。

## 完整真机归档已经通过

目标仍为已有信任的 D2，iPhone11,8 / iOS 18.7.10 / 22H374。
两次独立请求都带设备提示 1 MiB、age1d、请求时刻之前 60 秒的 StartTime、
显式 512 MiB 主机硬上限和 120 秒总期限。没有修改手机设置或日志。

| 操作 | 原始记录 | 自然退出及独立验证 |
| --- | --- | --- |
| archive | attempt008 | exit0，32.76 秒；196,126,720 字节、19,153 个协议块；独立 tar 读取 554 条目，所有普通文件载荷长度完整 |
| collect | attempt009 | exit0，33.91 秒；传输 195,829,760 字节、19,124 个块；CLI 报告 545 条目，独立核对 346 个文件及 194,891,646 字节解压内容 |

两次请求的目录和内容可以随设备日志变化，不做相同归档或逐字节相等的断言。
日期和大小字段是发给设备的请求，不据此宣称每个返回条目都满足时间筛选。
实际归档约 196 MB，先前的 32/128 MiB 是验证预算不足；attempt006 的 128 MiB 限额失败仍保留。
512 MiB 是本轮有界预算，不是新的产品限制，也没有移除已有传输及解压防护。
原始归档、解压文件、文件名清单和内容哈希仅留 private，不入 Git。

## RST 候选按既有取消契约结案

上一轮把运行时取消后的严格 EOF 失败列为产品候选，缺少契约依据。
[2026-10-07 原有验收](cli-tunnel-native-retirement-2026-10-07.md)明确记录：
实际代理子任务已经进入后，取消关闭可观察到 EOF 或 reset。公开 shutdown 契约要求
在预算内停止并 join 原 worker/子任务，而不保证任意 TCP 客户端完成 FIN 握手。
原有 Windows ConnectionReset 仍是真实错误，没有被改写为 EOF，也未声称解释或消除了它。

本轮只修改测试的错误假设，并将两种场景分开：

- 取消：保持真实 TCP SYN/端口前置证明、四个原 worker/子任务 join、duplex 严格 EOF、
  任务计数及原资源退款；TCP 结果分别记录 EOF 或原 ConnectionReset。
  超时、非零数据和其他 I/O 错误仍失败，不宣称完成业务传输。
- 自然结束：新增真实 loopback 对照，必须完整收到 `hello`，原转发任务正常结束后严格 EOF。
  原 Linux L04 资源基准中的 strict EOF 门槛未修改。

没有为一项不适用的 FIN 保证增加产品层 Drop guard 或延时。
相关源码在 `userspace/launch_tests.rs` 和 `userspace/connect_ack_tests.rs`；
契约复核见外部归档 `reviews/lifecycle-continuation.md`。

## 验证及剩余边界

attempt002：470 项核心测试通过；attempt003：108 项 CLI 测试通过、3 项原有忽略；
attempt004：构建成功。Cargo 调用串行执行，源码冻结前后哈希一致。
attempt010/011 各重复运行 470 项默认并发测试并保留取消结果，均通过；
两轮实际观察均为 EOF，没有据此捏造发生 reset 的新次数。

早期 rustup/venv 包装调用的 leader 虽 exit0，严格捕获器随后执行了 Job 残留清理，记录原样保留。
最终核心验证采用直接运行的 attempt010/011，CLI 采用 attempt014，实际 Cargo 二进制重验构建采用
attempt015，均无强制清理；归档实测 attempt008/009 也均自然完成。attempt017 的目录校验观测到
leader 退出后 Job 即时仍计 1，随后在 2 秒完成观察内自行归零；未把该主机记账时序判为产品泄漏。

当前 debugproxy 仍缺失，attempt012 明确报告前置条件且不输出脚本，不能记为正向调试通过。
WebInspector 设置/受控页面、完整壁纸当前端点、真实 HCI 与手表事件素材仍按前轮结论保留；
F02/F04/F06 先前搁置范围没有扩展。没有为了这些外部条件添加替代数据或修改设备状态。

原 companion/symbols 等已修复文件保留，本轮仅对实际问题作最小调整；
所有本轮进程及归档暂存均已清理，成功归档作为 private 交付保留。
既有 ios PID36976 未触碰；没有新增配对、DDI、kernel tunnel、HID、设置、app 操作。
未提交或 push。
