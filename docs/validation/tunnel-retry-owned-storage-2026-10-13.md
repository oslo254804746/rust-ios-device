# L02 retry/history 自有存储准入 · 2026-10-13

本次交付覆盖生产 reconnect ledger 的可证明自有存储，不将 L02 标记完成。
它补上创建端合同中的 retry/history 容器、原错误胶囊、调用方包装和删除返回 owner，
不声称任意原错误或依赖私有 allocation 已有完整字节上限。

生产构造器 Ledger::with_metadata 接入现有 global known metadata bank，按
size_of::<Ledger>() 持有真实自有 heap payload 的 byte owner。rows、causes 和
retired history 统一使用实际 Vec capacity 可见的 KnownVec<Box<Removed>>。
扩容时旧 backing 的 permit 保持有效，replacement 的实际 capacity × element
size 先准入；成功后才移动原元素。原 keyed lookup、身份、ticket、version、slot
及 episode policy 均保留；Vec 的行顺序稳定。

Ledger::start 位于 Source 第一次 poll 之前，预先创建并准入：

- rows/causes 的实际 backing；
- episode 和 cause 的 Box<Removed> inline payload；
- row 的实际 key capacity、route capacity；
- FailureReceipt 的 inline payload 与实际 copied key/identity route capacity；
- Original 的 inline OnceLock<anyhow::Error> payload；
- 调用方 SourceFailure 的 inline payload reservation。

这些费用成功后才发布 pending cause、更新 version 和 attempts。准入失败不消费
episode attempt、不创建当前 failure cause，也不启动 Source；已准入但仍保留的
空 Vec backing 继续带着它的实际费用，清理或 ledger 最后 owner 释放后退款。

capture_source 将取得的同一个 anyhow 对象移进预先创建的 OnceLock，不把原因
格式化、采样、替换或因 quota 拒绝。first/latest 共享 Original Arc 和 byte owner；
新 receipt 的 first 共享旧 first，latest 持有本次原对象，failures 同步累计。
SourceFailure 包装取得预先准备的 permit，原 source() 链以及 IO downcast 地址
保持原样。没有使用 Arc/anyhow 私有布局或 unsafe 初始化。

pending cause 在 clear/close 下保留它的原 node；同 poll 获得的 stale error 将这个
node 改为 retired，不重新复制 key、不增长集合、不请求新 byte fee。成功或没有
取得原错误的 interrupted attempt 只释放自身 exact pending reservation。

删除返回使用每个 row/cause 创建时即持有的 Box<Removed> node，组成无需 allocation
的 RemovedBatch。删除方没有 acquired-original 后的新 Vec backing 准入。
reconciliation 为 node 记录原索引，deadline 失败时重新插回保留的 Vec backing，
恢复相同行顺序、ticket、version、slot 和原错误对象。返回 batch 的实际 node
payload owner 随 node 移动，drop 不发生递归长链销毁。空 table backing 在不再
需要 rollback 时释放。

payload_receipts 的诊断快照使用收费的实际 KnownVec root backing。采样准入失败
由 caller 将 census 标记 truncated；它不删除 durable receipt，不伪装为完整测量。
诊断 Error::source 回调不在 ledger lock 内运行。

新增 reconnect/tests.rs 覆盖一字节不足的创建端拒绝、零剩余 byte 下的 stale
original 历史落地、first/latest 原对象指针与最终 caller owner 退款，以及零 quota
下的真实 backing/node rollback 和相同顺序/version/root。执行与格式检查由 root
统一串行运行；本子任务没有启动 Cargo/rustc/maturin 或设备发现。

未闭合范围包括 Arc 私有 header/allocator rounding、anyhow ErrorImpl/backtrace、
任意 dyn Error 私藏 heap/共享可变分配、std IO 和其它依赖私有 allocation，以及
原合同中 native shutdown/report/runtime packet/socket/client 等创建端范围。
size_of 自有 payload 及已持有的实际 String/Vec capacity 只是这部分真实 owner
合同；它们不代表整个原错误的上限，也不被 opaque count、固定槽位或样本取代。

Root 串行验证：4项新retry测试通过；受影响tunnel整组433 PASS / 0 FAIL /
2既有ignored，包括新增quota census不完整标志与原对象/原native join/严格EOF
断言。严格CLI及Core全feature/all-target Clippy、CLI build、fmt/diff通过。
实际只读manager 1项通过且Source连接为0；主机runner 5 PASS / 22 NOT_RUN /
0 FAIL，未执行真实设备发现或操作。其它平台、SDK/feature/MSRV未重跑。

统一构建同时含独立F04待提交源码，不能称为本提交独有二进制；最终combined HEAD
与全部552个build-source的raw Git blob/source/binary关联在外部证据登记。
外部目录为`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair`，
retry原始日志在`l02-retry-storage`，最终统一构建在`f04-native-closure/verified`，
最终源码冻结和提交proof在`f04-native-closure/final-build`。失败制品保留。
