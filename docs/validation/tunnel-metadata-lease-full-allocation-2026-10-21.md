# L02 metadata lease full allocation before Source · 2026-10-21

两个独立 metadata Lease 原来只计量两个 permit 字段，仍在 Source 完成后用
标准 Arc 创建。本项将 shared_value 改为应用声明的 Allocation<T>，包含实际
strong counter、原 T 及其对齐/padding；不估算标准库私有 Arc header，不新建
费用 guard 或预算池。现有封闭 owner 仍无 Weak/raw/mutation/pin 出口，T:Unpin；
最后 strong 原子 Release/Acquire 后移动原 T，释放整个原 backing，再析构原值
及其费用。所有 clone 共用原对象地址；Send/Sync 保持原值的线程安全约束。

PreparedPair 在 Source 构造/轮询及 retry start 前，先取得两池各自完整的两个
root 费用，再分配两个实际空目的地。任一原 bank 不匹配或整份 body grant
短/长都在任何 backing 前拒绝。取消或后一个分配失败时，空 backing 先释放，
原 own/cross 费用随后退款。成功路径只将原字段 grant 合并到各自已付费的
own body，再把另一池的 cross body 移入 Lease，没有 Source 后的新 root 分配。
两池关闭、零可用字节时也能原位完成。两个 root 各自保持原 bank、字段费用与
num_permits 含 own body 的语义；各自最后 owner 独立向两池退款。

managed_metadata_bytes 的发布额度继续计入完整两个 root。后续 global 与
publication 字段准入分别减去一次已独立持有的 pair 费用，避免重复扣除；原
route/native/RSD 独立费用扣减保持。table、native Registry、RefreshProof
仍共享各自原 Lease，原 handle、epoch、absolute deadline 和 Source 错误不变。
实际 native metadata/freshness 夹具无需调整，仍保持零余量、原 typed IO、
严格 Ok(0) EOF、原 joins 完成与 table/外部 owner 分离退款。

原 shared_value 4项保留并改用完整实际 layout：128对齐、原 String 地址、
8线程最后 owner、原析构展开和零尺寸值的实际 counter backing。新增 Lease
2项验证恰好两个 backing 在两池整份借记之后产生、闭池 finish 零分配、原值
位于对应原 backing，以及真实 dealloc hook 发生在任一原 own/cross 信用返还
之前；另测未使用目的地取消和任一池差一字节时零 backing。原 Lease 3项继续
验证独立根/闭池/8线程、原 bank/地址及最后退款，并增加长 grant 零分配拒绝。
新增 manager 2项直接追踪最终发布的两个原 Lease 地址确实来自 Source 调用前
的 backing，及差一字节拒绝时零 Source/零失败记录/零 receipt/原生表空与退款。

第三轮14个命令全部通过（含CDP browser lease9）：observed 边界1、shared value4、Lease5、manager2、
native metadata7、native freshness14、strict CLI all-features/all-targets
Clippy -D warnings、完整CLI1750 PASS / 0 FAIL / 13原ignored、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。未执行 Miri、其它平台或真实设备验证。Core/Cargo
未改，复用 e8b5129 的既有Core证据，不声称本轮重跑。

第一轮完整CLI1749 PASS / 1 FAIL / 13原ignored：旧 observed-route 夹具只留下
publication destination 与 host-minus-one 的费用，新增 lease 前置准入先拒绝。
生产算法不因该失败改变；夹具只加上实际 Lease::pair_bytes 前缀，仍恰好差
一个 host 字节，新增观察器确认两处原 body 已在此零余量边界先付费。原
observed Admission/ResourceLimit、零 Source/USB/paired、native空与部分/最终
退款断言全部保留，无放宽池额度、忽略或删除测试。第一轮日志保留；第二轮
定向边界通过。

第二轮完整CLI1749 PASS / 1 FAIL / 13原ignored，失败为既有CDP浏览器
reload-during-detach夹具：80ms flush延迟可能在实际HTTP reload之前完成，
进而合法发出detach回复，而released helper严格要求下一条消息为关闭。该CDP
生产路径不使用本项metadata Lease。未放宽关闭/消息/EOF/原flush次数断言；
仅为测试Source增加显式AtomicWaker flush gate，此单项夹具在实际reload后
才释放，并在reload后再次确认原flush仍未完成。展开guard保证测试失败也释放
gate；原80ms delayed-flush及cleanup timeout测试继续保留。第三轮先跑全部
CDP browser lease9和原observed边界，再跑本项全部门禁及完整CLI，通过后提交。

684个冻结源码与当前受测版本一致，10处源码差异；实际host报告与归档CLI的
SHA-256一致：`6dae978828f840cee62cd3aef15d79fd50f56f88104adf9f7b79a91caa330137`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-metadata-lease-full-allocation\round3`。

其它 marker/private headers、原 opaque Error/Anyhow/context、SDK/发现输出、
HTTP/WS/配置/凭据、框架IO/task/allocator与完整freshness仍按原范围审计。
128MiB是已准入已知对象池，非完整manager heap/RSS。L02未关闭；主项23交付/
6缺开发/L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、
push或发布。
