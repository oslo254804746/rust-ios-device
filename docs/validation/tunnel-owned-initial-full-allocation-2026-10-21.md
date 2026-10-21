# L02 owned initial RSD actual allocation · 2026-10-21

OwnedRsdHandshake 原先用标准 Arc<Directory>，费用只含 Directory 本体，
不含私有控制块；连接 helper 又在 bootstrap 后才准备 owned 根。本项使用
声明完整布局的 Allocation：AtomicUsize 与 ManuallyDrop<Directory>。
原 Directory._body 费用覆盖实际完整根，无第二份费用 guard、不估计标准
Arc ABI。公开 clone、service view、iterator、解析与查询接口保持，不导出
Weak、raw owning pointer、pin 或可变缓冲区。

prepare 用原 bank 的精确 whole grant，在任何根 backing 前校验 bank/尺寸，
随后实际容量一根原位存放空目录。生产 owned connector helper 在身份 future
首次 poll 和首次拨号前准备该根，四种原 bootstrap 模式共用它。成功读取只
填原根，没有晚期根分配。原身份查询后的 whole timeout、各模式 timeout、
fresh transport、ResourceLimit final 与原最后错误规则保持，无新增期限、
任务或 bank。

公开 owned_handshake_on_framer 保留 RsdOperation::begin 的原错误优先级，
在首次 owned 读取前准备根。公开与预备根入口共用原 operation guard、
borrowed read、progress 与 finish 路径，并校验原 bank。private fill 只在
唯一强引用下修改；required 字段优先级、legacy 空 UID、零端口、无效条目、
重复项、feature 与六消息限制均保持。clone 使用原强引用计数，最后 Release/
Acquire owner 移出已知 Unpin 原目录，先 dealloc 同一完整根，再析构原字段
和费用。known_bytes 包含该实际根及原字段容量。

新增两项根测试：foreign/短/长整笔 grant 与 N-1 在零 backing 下拒绝；
exact N 首次实际分配前已扣完整费用，Directory 位于原根内；闭池零额度的
service/iterator/clone 保留原根与字段地址，实际根 after-free 时任何原字段
或根费用均未退，最后精确退款。原八线程最后 service export 测试继续通过。
旧字段 N-1 测试只将 body 公式更新为实际完整根，没有扩大固定额度。

另新增三项协议测试：根拒绝先于 identity/connector poll；身份等待中 caller
drop 释放预付根且不拨号；实际 duplex bootstrap 等待中取消关闭同一 socket，
peer 严格 Ok(0) EOF 并实际 join 后核对退款。既有四种 owned 初始握手成功与
六次语义失败后换新 transport 的 fixture 强化为验证首次 connector 前整笔
根分配、全过程仅一个根、最终实际根地址相同，保留严格 EOF/join。

第一轮仅执行 XPC 编译，新测试 drop((tree, wire)) 违反借用规则而失败。
进程终止后仅改为先 drop(tree)、再 drop(wire)，第一轮日志保留。第二轮
19 个命令全部通过：XPC 216、existing 22、
prepared 12、activation 9、proxy
4、tunnel 131，无 mDNS initial
18、owned current 5、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。

全部 Cargo 与 build 由主控串行执行；本轮 Core 生产源码改变，使用实际
Core/CLI 结果。685 冻结源码无漂移，四处源码差异；host 受测与归档 CLI
SHA-256 一致：
7dab4f243a0aab76aa09ef1fcd95259440fb8d377f852ccd0611713e001642ad
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-initial-full-allocation\round2
未声称 Miri、其它平台或真实设备验证。

legacy raw Arc/Weak、opaque 错误、其它 marker、SDK/发现与 service metadata
副本、HTTP/WS/配置/凭据、框架 task/allocator、完整 endpoint 恢复继续原
范围。128MiB 为已准入已知对象池，非完整 heap/RSS。L02 未关闭；23交付/
6缺开发/L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、
push 或发布。
