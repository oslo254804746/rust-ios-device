# L02 owned prepared RSD actual allocation · 2026-10-21

PreparedRsdOwner 原先使用标准 Arc，catalogue 的费用不能覆盖或证明其私有控制块
的完整布局与实际释放。本项改为声明完整布局的强引用根：AtomicUsize 与原
PreparedRsdCatalogue。公开 Clone、Deref、Debug、ptr_eq 保持；不导出 Weak、
raw、pin 或可变入口。原 catalogue body grant 支付实际根完整布局，不增设
费用 guard、第二份 body fee，也不猜测标准 Arc 私有 ABI。

owned prepare 在原绝对 Clock 的首次有效 poll 内、identity/dial 之前，先用
原 bank 的精确 whole grant 分配实际空根。foreign、短、长 grant 在 backing
创建前拒绝；零期限不分配、不查询身份、不拨号。同步分配后再次检查原期限。
同一空根贯穿原四种 checked wire 模式；只有有效 Handshake 才消耗目的地。
原 connector refusal 仍立即返回；已连接后的可重试 bootstrap/wire 失败才进入
下一模式，最后原 IO 与原绝对期限保持。

required UID、Services、所有 Port 验证与原字段复制、排序、容量上限保持。
共享 CatalogueParts 将 fallible 字段存储与实际空根分开；字段失败或 caller
drop 时，字段先释放，根真实释放后才退 whole body fee。最终选择按 Vec 实际
capacity 计费，全部检查完成后才移动原费用并原位填入根；该阶段没有新的
backing 或 bank admission。最后强引用以 Release/Acquire 获得唯一权，移出
已知 Unpin catalogue，先释放完整声明根，再析构原字段与费用。

新增四项根测试覆盖精确整笔准入、foreign/短/长零 backing、部分字段失败、
闭池零额度下原位完成、原根/UID/rows/mode 地址、实际根 after-free 信用点及
八线程最后 owner 精确退款。新增四项 existing RSD 测试覆盖根拒绝时零 IO、
零期限、实际 duplex pending caller drop 的严格 Ok(0) EOF、四次 wire 重试
只用一个根，以及 connector refusal 单次拨号。原 owned current 同端口与
变更端口真实 loopback 服务重启测试继续通过。旧 raw Arc API 保持，其私有
header 与 Weak 生命周期仍属原范围待处理。

第一轮新测试误把 connector refusal 当作四模式可重试失败，existing 测试
20 PASS / 1 FAIL，进程终止后修正测试：关闭已连接 duplex peer 触发原 wire
重试，另加单次 connector refusal 断言。没有修改生产 retry 分类或扩大额度。
第一轮日志保留；第二轮 18 个命令全部通过：

- existing RSD 22、prepared RSD 12、XPC 210、activation 9、proxy 4、tunnel 131。
- no-default 与 tunnel-only check；无 mDNS owned 5、proxy 5。两项 check 有既有
  dead-code warning，不将其称作严格无警告检查。
- Core 和 CLI all-features/all-targets Clippy -D warnings；完整 CLI
  1765 PASS / 0 FAIL / 13 原 ignored；CLI build、fmt 与 diff check。
- 实际只读 manager 1 项（Source/usbmux trap 为 0）；host 5 PASS / 0 FAIL /
  22 NOT_RUN，real_device_executed=false。

全部 Cargo 与 build 由主控串行执行。Core 生产源码已改，本轮使用实际 Core
结果，未借用旧 Core 证明。685 份冻结源码无漂移，七处源码差异；实际 host
报告与归档 CLI SHA-256 一致：
`a67151a5c2542690c488ec57e1d9ae1f36762391faa223944c686aeb39ee022b`。
日志、source delta、verified-ios.exe、current 与提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-prepared-full-allocation\round2`。
未声称 Miri、其它平台、真实设备或新增 owned 四模式成功 fixture 的证明。

initial owned RSD、legacy raw Arc/Weak、opaque 错误与其它 marker、SDK/发现、
HTTP/WS/配置/凭据、框架 task/allocator、完整 endpoint 恢复仍继续原范围。
128MiB 为已准入已知对象池，非完整 heap/RSS。L02 未关闭；23交付/6缺开发/
L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
