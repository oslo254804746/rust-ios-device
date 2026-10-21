# L02 owned prepared four-mode success · 2026-10-21

在 57137aa 完整 owned prepared 根实现之后，补齐公开 owned prepare 调用在
Direct、Queued、Legacy、Passive 四种 checked 模式各自成功的协议测试。
复用原 raw H2 fixture，IPv6 ::1 临时 TCP listener 与显式 UUID，不查询宿主
身份，不 poll CLI NativeSource，不执行真实设备操作。

每种模式前面的已连接 peer 关闭，最终 peer 完成该模式原 bootstrap，辅助流
与分片 Handshake 走原解析器。观察实际分配：首次拨号前整笔根费用已扣，
只分配一个实际根，最终 catalogue 位于原根内。成功选择严格等于原模式前缀，
Vec capacity 为四；原 bank 与 known_bytes 相符，所有 scratch 费用已退。
相同 peer 只有严格 Ok(0) 才结束并实际 join。闭池零额度下保留克隆，原 UID、
feature、mode Vec 地址不变，最后 owner 结束精确退款。

本项只有一处测试源码改动，生产代码和受测 CLI 二进制均与 57137aa 相同。
九个验收命令实际通过：XPC 211、无 mDNS owned 新测试 1、Core/CLI 严格
all-features/all-targets Clippy -D warnings、CLI build、fmt、diff、实际只读
manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。全部 Cargo/build 由主控串行执行。

685 冻结源码无漂移；实际 host、归档与前次已全量验证 CLI SHA-256 一致：
`a67151a5c2542690c488ec57e1d9ae1f36762391faa223944c686aeb39ee022b`。
完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored 复用前次实际结果，未声称本项重跑。
证据、source delta、verified-ios.exe 与提交后 rawGit/worktree proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-prepared-four-modes\round1`。

initial 根、legacy raw Arc/Weak、opaque 错误及完整 endpoint 恢复等仍继续
原范围。128MiB 为已准入已知对象池，非完整 heap/RSS。L02 未关闭，主项
23交付/6缺开发/L02待全审计、有效30与Q04全部排除不变。仅本地提交。
