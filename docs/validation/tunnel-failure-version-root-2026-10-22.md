# L02 retry/failure generation complete root · 2026-10-22

原 retry episode、failure row、Source receipt 与 manual-success 待处理元组
共同保留 Arc<()> 失败版本，其不透明控制块未计费。本项引入私有 FailureVersion，
复用既有已验 shared_weak_allocation::Strong<(), ()> 的完整声明根：strong/
implicit weak、unit 值和原费用一并按实际布局从同一 bank 准入。不估算标准
Arc 私有布局，不新增 unsafe、不新建生产 bank、不扩大固定128MiB额度。

retry.start 在推进 episode/次数或首次 Source poll 前准入原新版本；Node::input
在消耗 Input 唯一 outcome key 前完成原版本根。FailureReceipt 接受同一原版本
clone，并在其它 capsule/selector backing 前核查版本费用来自同一 bank。
Episode、FailedTunnelAttempt、receipt、failure stamp、pending/manual success、
rollback 与恢复筛选仅改用同一根的 clone/原身份比较，保留原父对象、旧版本、
失败许可、原 IO/Source 指针与 typed downcast 链。全局/device/desired intent
版本仍是独立原范围，未机械转换其 stop/clear/reset 控制路径。

新根费用独立于 row 或 receipt body，最后真实持有者结束才可退款；测试中的
Weak 继续保活完整 backing，在最后 strong 后升级失败，最后 Weak dealloc 后
才退款。生产接口不导出 raw/Weak；cfgtest 原版本指针/Weak 观察对应改为同一
声明根的等价观察。既有通用 Weak 根仅扩大 cfgtest after_free hook 可见性，
其原位 Drop、计数、最后物理释放和费用算法保持。

新增5项测试：N-1 与 foreign/short/long 整笔授权在 backing 前拒绝；实际
exact-root allocation 前全额扣费、原 layout/unit 字段地址、闭池 clone 的同一
根与 A/B 隔离、最后 Weak 实际 free 后精确退款；并行原 clone/Weak 不复活且
只退款一次；foreign-version receipt 在全部 capsule/selector 分配前拒绝并
释放原参数；重试版本最后一字节拒绝时原时钟/首次尝试/整段deadline/下一次
期限/次数/失败数/phase/class/sequence、原回执和原 IO 指针完全保留，清除表后
最后调用方 report 仍保有原根与 IO，再结束后全额退款。

原 exact fixture 公式只增加真实 FailureVersion root bytes；不改变生产固定
额度。Node 构造观察新增第三个实际根的布局、原字段地址、原 bank 和不同
backing；snapshot、retained receipt 和原版本观察句柄的根费用按实际寿命保留。
R1 strict/new-root3/weak-root4通过，retry45通过/2旧退款公式失败；仅精确补上
retained receipt 的32字节实际根与3个观察版本的96字节，并断言结束观察前
费用仍扣、结束后精确退款。R2 retry47通过，failure-table11通过/2旧公式失败；
仅补第三根实际分配证据与snapshot仍持有的32字节。生产逻辑自R1至R4保持，
未删除退款/原指针断言，日志保留。开发期生产check通过，all-targets检查所报
旧 Arc 类型测试适配错误已逐项处理，最终严格检查见下。
R3前七项门禁均通过，完整CLI1766通过/4旧测试公式失败：outcome key原构造
观察仍按2根而新增实际第3根、success intent结束多退原版本根32字节，两个
scan精确窗口仍通过cfgtest Node::input_bytes漏算原版本根而提前耗尽。仅补
该helper的实际完整根费用、第三根布局/原bank/原地址/独立backing及intent
真实退款；仍保留原key地址、duplicate拒绝、Source未联系和paired等待阶段。
R4加入这两个实际失败范围的定向门禁；未扩大生产固定额度，R1/R2/R3日志保留。

R4 全部15个命令通过：CLI all-features/all-targets Clippy -D warnings，
failure version 3、原 Weak root 4、
retry 47、failure table 13、intent
10、manual success 13、outcome key
4、global metadata 17、完整 CLI
1770 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、实际只读 manager 1
（Source/usbmux trap0）、host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。全部 Cargo/build 由主控串行执行。

691 冻结源码无漂移，25处差异全部在CLI，Core/Cargo.lock未改；Core证据沿用
已提交846c533的watch10/无mDNS10、tunnel159、owner42/无mDNS37及其余29门禁，
并以冻结哈希逐项证明全部Core源码未变，未声称本轮重新执行Core门禁。
本轮实际 host 与归档受测 CLI SHA-256：
92bd2aeac0399aa2fb8cbfa11a21ff6683ad5dcef82d64910d704332b4750b78
source delta、verified-ios.exe、current/提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-version-root\round4

global/device/desired intent/scan marker 和控制路径、其它 watch、公开 native
Arc/Weak、opaque error/report/wrapper、SDK/发现/metadata副本、HTTP/WS/配置/
凭据、框架/allocator/waiter/Waker及完整各模式endpoint恢复继续原范围。
128MiB 是已准入已知对象池，非整个 heap/RSS。未声称 Miri、其它平台或真实
设备验证。L02仍待全审计；主项23交付/6缺开发/L02待全审计、有效30与Q04
全部排除不变。仅本地提交，无push、发布或真实设备操作。
