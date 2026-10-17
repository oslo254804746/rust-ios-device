# L02 prepared outcome key destination · 2026-10-17

原 Input 仅预留一份 UDID 长度费用，Node::input 再 String::clone 后检查容量；
费用还留在 Input 内。本项在 Input 工厂中先按实际 u8 Layout 准入并创建空 key
目的地，替换这份预留而不重复收费。Input 新增的 inline mutex/目的地槽按实际
sizeof 计入原 body。copied、USB 原字段转移和 cfg(test) moved-input 路径均接线。

Input::outcome_key 只取用一次，先检查原 bank、实际空缓冲区容量及 grant 大小，
取出后释放锁，再填入 UID 字节；String::from_utf8 保留同一原指针，无扩容或
第二次目的地分配。重复准备明确 ResourceLimit，不用新 bank 或未收费 fallback。
Node 仍先准入原 body/identity，然后取该 key 和独立费用，存入已有 key permit
字段；into_success_key 将同一 String/permit 移给原 SuccessIntent。Input owner
和 key owner 可分别结束，最后一个实际 key owner 才退款。

原字段/独立 grant 的 Drop 顺序保持。构造局部 storage 先于字段绑定；USB
先完成 key 准入再拆分原 snapshot 字段费用，拒绝时原 raw grant 不变。移动
入参用独立局部变量承接，确保部分失败中 String 先析构再归还本地费用。没有
任意析构或等待位于 key mutex 内。原 raw USB UID 指针、失败表回滚、success
key 复用、原 Source typed cause、期限/epoch/native lease 和 joins 保持。

candidate_input 专项 11 PASS，新增 4 项覆盖：原空目的地在零余量下填充及
单次取用、空 UID/原 pointer/容量/独立 grant；Node 同指针和重复拒绝不泄漏；
实际 SuccessIntent 在 Input/Node 消失后保留同一 key/费用，零余量时最后释放；
USB 差一字节拒绝保留原 raw grant。原文本测试增加真实空目的地分配及其部分
拒绝边界，不放宽旧 assertions。

首轮专项 11、完整 tunnel 671 和 upstream 10 通过，随后严格 Clippy 拒绝
3 处用于 Drop 顺序的同名重绑定。改用 incoming 参数和独立局部名，保留原
析构顺序，没有 lint allow。最终轮重新运行完整门禁：tunnel 671 PASS / 0 FAIL /
2 原 ignored，upstream access 10 PASS，all-features/all-targets CLI Clippy
-D warnings、build/fmt/diff、实际只读 manager 1 项通过。Source/usbmux trap 0，
host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。首轮日志保留。

647 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`33aaec1759033ee61067adb868e4b451470093557c6f0a0c1b84cbb30e749357`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-outcome-key\round2`。

Identity/retry/receipt 原 selectors 的普通 clone/事后容量检查、发现/SDK 输出、
初始公开 RsdHandshake 目录逃逸、HTTP/client/config/framework/private IO、其他
原 allocation 的最后 owner 和完整 freshness/刷新/复用/重启/manual epoch
审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/
6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
