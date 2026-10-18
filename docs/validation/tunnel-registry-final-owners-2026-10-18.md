# L02 native registry final owners · 2026-10-18

Registry 原 body/slots 已在创建前准入，但 body permit 位于 raw Arc payload
内，不能证明 allocation 结束后才退款。完整引用审计未发现该私有根的 Weak、
pinning 或 raw Arc 出口；本项统一采用封闭 RegistryOwner，覆盖 manager、
standalone、scoped、Ownership、Reservation、NativeLease 和 RefreshProof。
所有克隆消费原 Arc，最后 allocation 先结束，再执行原 Registry payload
析构及 body/slots 退款。原 Drop 仍只请求 stop，不据此声明成功 joins。

原 sizeof<Registry> 和实际 slots backing 费用、准入顺序、固定槽位、issued/
incarnation、stamp、Source 前 reservation 和刷新/退休规则保持。reserve
使用稳定的 owner 专用实现；测试 get_mut 保留唯一性，既有精确 rebind
仍先取得新 grant 再释放旧费用，原 slots/namespace 不变。没有新 sidecar、
额度 padding、private Arc header 估算或额外任务。原 core native handle Arc
与所有身份/IO 指针、实际 join、strict EOF 断言保持。

新增2项：关闭银行/零余量下8线程最后 root 保持原 body/slots 指针，唯一
get_mut 与别名拒绝、最终单次退款；实际 userspace worker 的 NativeLease
超过外部 registry 寿命仍持有原根，闭池零配额下 lease Drop 先请求 stop，
原 drain 取得实际 joins、peer Ok(0) EOF 和 task baseline，最后 root 才退
body/slots。此 fixture 无物理设备或 OS TUN。既有 reservation 生命周期、
精确/差一字节、同一 backing 重配及刷新/原错误测试均在全组保留。

专项3通过（2新增和1既有），完整 tunnel727 PASS / 0 FAIL / 2原ignored，
upstream access10、all-features/all-targets CLI Clippy -D warnings、build/
fmt/diff、实际只读 manager1通过；Source/usbmux trap0，host5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。权限热更新本次完整回归也通过；
前项首次未确认原因的502日志继续保留。

664个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`e5a309e81c74e80b695538634910dc6d1fe803fd5e2170f63c64d3c5c970d131`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-registry-owner\round1`。

Freshness/Publication 等共享 owner、private header/opaque error/IO、发现/
SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness审计继续。128MiB为
已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与
Q04全部排除保持。仅本地提交，无真实设备、push或发布。
