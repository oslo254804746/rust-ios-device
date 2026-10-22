# L02 marker cross-bank finite census · 2026-10-22

本项把先前只读报告的跨bank假设落实为已执行的有限审计fixture。
唯一源码变化是在cfg(test) retirement/tests.rs末尾增加一项测试，所有
原测试逐字保持，retirement/lifecycle/其它生产源码及Core/lock全部未变。
这不修复marker分配，也不完成L02、arena容量或所有retainer的证明。

原Lifecycle A仅给足既有watch_bytes+initial_metadata_bytes(KEY)，暖身
取得并丢弃Ticket后可用额恰为0，然后只关闭其Semaphore，保持Lifecycle
生产者存活。同一个KEY四轮取得当前Ticket，在原commit fence内把其
OwnershipStamp交给各自独立、有限bank的Registry B_j。每轮drop Ticket、
clear、用原old global做该B_j的retire_global并drop old；最终额外Ticket
触发最后一次Cell/device watch lazy reset，再drop witness。

实际断言：

- A在四轮及最后lazy reset期间一直closed且可用0，仍只有一个canonical
  gate；四个B各付其原实际backing和精确incarnation费用。
- 四个实际Entry保留八个不同的global/device Arc身份，所有配对及跨字段
  Arc::ptr_eq均不同。借用原字段检查时每个strong_count恰为1；无额外
  Ticket、Snapshot、watch receiver、Weak或clone保留旧pair。
- 这些Entry的handle/input/metadata/global_metadata/fresh_metadata/
  first_freshness/latest_freshness全None；没有隐藏的A grant路径。
- drop唯一Lifecycle A后，A原有已知费用全部返回，而借用同一B Entries
  仍观察到八个distinct旧Arc且strong1。当前raw Arc控制块因此没有保留
  这些A费用；没有猜std Arc私有header大小。
- 分别drop原Reservation使空entry移除并准确归还B incarnation费用；
  drop各Registry后各自bank精确全退。未创建/连接/attach任何native runtime，
  无Source/IO或join主张。外部有限fixture Vec不是A付费keeper。

这是对“所有历史raw marker keeper都已持有原producer bank A正费用”
这一前提的实证反例，不是生产State同G路径的无限轨迹，不是N=4超过
某个已批准arena上限，也不是完整计数。当前A全退断言只记录raw Arc现状；
未来arena必须改为其物理backing整笔费用随最后marker/Weak keeper保持，
不能把这里尚未支付marker控制块的退款当作应保留的新contract。独立
Registry兼容、真实旧身份和原control语义仍需保持。

round1全部7门禁通过，所有Cargo由主控串行执行：

- cross-bank-census：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 0.00s
- registry-contract：test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 1831 filtered out; finished in 0.02s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1824 filtered out; finished in 6.68s
- reconnect：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1806 filtered out; finished in 6.84s

另含strict CLI all-features/all-targets Clippy -D warnings、fmt/diff。
708冻结源码无漂移，仅上述1个测试文件追加；所有生产、Core和lock hash
与1501af2保持。此次未重新跑完整CLI/build/host；直接前序1501af2已验
fullCLI1839/0/13原ignored与15门禁/host5 PASS、22 NOT_RUN只作为前序证据。
生产二进制仍为该轮实际host所测、已归档的同一文件，SHA-256：
f015f4dd269d16c72860471038184e87eb0cad9659f62357f7a7e1c954e2da4a
当前实际执行测试二进制target\debug\deps\ios-ca16b908d2a6004a.exe的SHA-256：
41ba6d5d476b83f819dbcbd0869fd4efa741f066cebce41a305b7bc9958043b2
命令/log/result、source delta、当前/提交后rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-marker-cross-bank-census\round1

独立设计报告逐字段追踪lazy reset和paid_watch当前值，主控已执行上述
fixture并验证readonly review指纹。报告不代替主控执行记录。完整marker
census、所有跨bank/producer/retainer/Weak/control peaks、不可变容量
authority、全backing寿命以及opaque/public/legacy/SDK/framework/transport
和全部mode endpoint recovery/native exposure仍是原scope。本项不选
任意cap、不引入numeric identity、不豁免foreign/private/legacy边界。
128MiB为已知对象准入池，非整个heap/RSS；无真实设备/Miri/其它平台验证。
有效30/Q04全部排除、23交付/6缺开发/L02待全审计不变。
