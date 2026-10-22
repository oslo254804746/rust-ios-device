# L02 foreground connection semaphore root · 2026-10-22

原NativeScope连接许可使用独立Arc<Semaphore>，它可由返回的owned permit
保留到Scope Inner结束之后。a2cc312的Inner共享根和完整connect F均不支付
这个独立分配。本项把同一个Tokio Semaphore值放入已知完整布局的
Shared<Semaphore>，由原Registry byte bank支付其控制/值/permit/padding，
不猜std Arc私有header。新ConnectionPermit只持有独立Slots根和私有count，
非Clone、不持Inner，保留最后Scope Drop先关闭原Registry的行为。

取得仍调用原Tokio Semaphore::try_acquire_many(count)，仅将成功的borrowed
permit忘记后立即交给拥有同一Slots根的ConnectionPermit。两者之间没有
await/分配/失败步骤。Drop先add_permits(count)再释放其根；与原Tokio
owned permit的固定4容量/NoPermits/Closed/0-count和线程间Drop保持。
Private ConnectionSlots提供内部Deref<Semaphore>；现有生产代码仅使用
非阻塞取得和许可归还，未新增等待任务/时钟/重试/容量。该内部接口不
等于对外开放一个新Semaphore，也不据此断言任意未来调用都满足容量证明。

NativeScope构造一次checked_add并准入Inner+Slots两份完整实际布局，精确
split原grant；两份空实际Destination均成功分配后，才构造Inner和原4许可
Semaphore并无失败地place。Destination复用既有AdmittedVec<Allocation<T>>
和Shared::place，同一原指针/layout，无额外shared wrapper allocation或
新unsafe。未使用的Destination通过values→storage声明顺序先free后退费。
第二份分配拒绝不会构造Inner/关闭独立持有的原Registry。预付后bank关闭
不妨碍原destinations继续place。

Inner/NativeConnection Drop、SDK具体连接调用、connect原prefix/body、
collect/close及原error retainer正文有独立相等guard（permit参数仅换为
私有ConnectionPermit）。原Shared clone/Drop/allocation实现逐字保持；
metadata_vec生产代码不变，只增加cfg(test)分配前拒绝hook。原6项frame
测试源文件不变，其实际F size通过类型测量随真实permit布局变化。

新增6项测试及原root3更新：

- 原同一个Semaphore指针/Slots根，3+1取得、NoPermits拒绝、3退款再取得、
  Closed拒绝及原4容量归还，整个permit操作没有受控新allocation或额外
  metadata费用。0-count/5-count及四许可跨线程Drop保持原容量。
- 原最后Scope结束时Registry已closed+empty，活着的owned permit只保留
  原Slots根与费用；Inner准确退款，Slots直到其最后实际物理free后的观察
  点仍持整笔费用，再精确退款。没有用Scope root把native stop推迟。
- shared Destination的foreign exact grant和同bank N−1 grant分别拒绝，
  零backing；精确grant只分配原完整layout一次，closed0 place保留同一个
  pointer/span/原值，无第二allocation，最终全额退款。
- 第二Destination确定性拒绝：实际两次分配尝试、仅第一backing成功；
  原whole fee已全持有，第一空destination被清理、最终退款且原Registry
  未sealed/occupied0。该hook不是实际OOM；未独立观察该unused Vec的
  dealloc时刻，free→fee顺序由实际AdmittedVec字段声明源码证明。
- 另一条实际构造在第二份backing成功时关闭原bank：两份backing仍按原
  预付完成place，没有第三份allocation；四许可可取得/归还，最终全退。
- 原3项root测试仅按真实双root布局/两个实际free点与退款更新，保留
  N−1 before-allocation/clone identity/实际cleanup错误持根/native joins/
  严格Ok(0)EOF/任务基线断言。无额外permit时error Drop退两根，有额外
  permit时专门测试只退Inner。跨Scope许可测试使用空Registry，其关闭
  时机源码与其它实际native回归结合支持，未冒称该测试启动了native。

独立只读设计/实现审查均未发现需修改的真实缺陷，当前5源码SHA及原
frame_tests SHA已复核；未运行审查者Cargo/测试或修改源码。本项无新unsafe/
SDK路径/真实设备操作，不完成其它marker或opaque范围。

round1首轮全部18门禁通过，全部Cargo/build主控串行：

- slots-root过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1838 filtered out; finished in 0.01s
- scope-root过滤：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1841 filtered out; finished in 0.01s
- shared-root过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1837 filtered out; finished in 0.00s
- metadata-vec过滤：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1834 filtered out; finished in 0.00s
- foreground-all过滤：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.03s
- ownership过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1836 filtered out; finished in 0.01s
- native-retirement过滤：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1734 filtered out; finished in 43.55s
- lifecycle过滤：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1815 filtered out; finished in 6.70s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1833 filtered out; finished in 1.56s
- http-control过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1839 filtered out; finished in 0.04s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1840 filtered out; finished in 0.00s
- cli-full过滤：test result: ok. 1831 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 246.09s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。706冻结源码无漂移，5处源码差异仅CLI（含1新增）。
Core/Cargo.lock与a2cc312/ec25全量hash一致，Core沿用ec25已验24门禁/
tunnel159/reservation2/initial1/activation5/无mDNS2，未声称本轮重跑Core。
实际host及归档verified-ios.exe SHA-256：
ba0803840e46aefe906e0e9e42b0f455fe2d64af63ed6c684209a2cb1065ac8b
命令/log/result、source delta/current/提交后rawGit proof和实际二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-foreground-slots-root\round1

Scope根及Slots根现分别支付真实allocation并覆盖实际所有者寿命，但原
run_tunnel_start另一standalone外F、返回device/SDK/credential/metadata/
opaque errors、public Core Arc/Weak/watch、HTTP/framework/task/transport/
allocator/waker/waiter、marker完整census/same-bank keeper/producer/Weak/
控制峰值和arena容量证明、所有模式endpoint恢复/native exposure事务
仍属原范围。未用private/legacy边界豁免。128MiB仍是已知对象准入池，非
整个heap/RSS；未做Miri/其它平台/真机验证。有效30/Q04全排除、23交付/
6缺开发/L02待全审计保持，继续原七主项目标。
