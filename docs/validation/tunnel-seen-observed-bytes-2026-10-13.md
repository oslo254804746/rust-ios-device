# L02 seen 与 paired observed 字节归属 · 2026-10-13

扫描 seen 不再复制 UID 或保留未收费 HashSet backing。真实 KnownVec backing
进入现有共享池，排序元素共享原已准入 Input 的 Arc 与 UID buffer。USB 在
coordinator 准入前保留该身份；coordinator 拒绝后 paired 同 UID 仍不重试。
manual 优先和 USB first USB / otherwise last 的原选择规则保持。seen 持有
Input 与 backing 到扫描结束，最后实际 owner 释放退款，不重复收费原 Input
已经预留的一个 outcome UID 副本。

完整 paired view 的 observed 改为原 UID/host 的排序位置索引，真实 Vec capacity
收费；包含所有原观测行，不只包含已选候选。它在 Complete 和任何新 paired
ticket 之前整批准入；拒绝按 Incomplete 处理，释放部分 raw/plan/observed，
保留已准备 USB window 和原发布。paired seen 的整个增长也在 ticket 前准入。
完整 view 仍只按 exact UID/host 处理 scan-start captured 的原 publication Arc，
incomplete view 不认证缺失，不建立 automatic ticket 或发布前缀。

新增六项覆盖共享原 Input/UID、coordinator 拒绝身份最后 owner、增长差一字节
保持原 backing/keys/费用、observed 精确额度/一字节拒绝与退款、生产 coordinator
拒绝后 pool 恢复但同 UID 不从 paired 重试，以及 observed 最后一字节拒绝时
保留旧 Mobdev2 publication 并继续发布 USB window。原对象、错误、epoch、权限、
strict EOF、原 task/native joins 与绝对期限断言未删减或放宽。

整组 tunnel 450 PASS / 0 FAIL / 2 原 ignored。随后严格 Clippy 要求去掉一处
显式自动解引用；该等价写法修正后受影响预算 14 项、严格 CLI Clippy、CLI build、
fmt/diff、实际只读 manager 1 项均通过。整组结果按影响复用，不称最终源码又跑了
一次整组450。只读 manager Source/usbmux trap 为0；host runner 5 PASS /
22 NOT_RUN / 0 FAIL，real_device_executed=false。

首轮整组447 PASS / 1 FAIL / 2 ignored：过早 drop seen 让原 candidate-index
额度拒绝测试越过目标准入并删掉旧 backoff；修复 owner 生命周期后保留原断言。
新增 coordinator fixture 初次配额落到 candidate backing 拒绝，改为 coordinator
payload 准入边界后通过；原 paired 调用/不同 UID 连接/同 UID 不连接断言保持。
初次 Clippy 失败及一次外部门禁命令参数错误的完整日志保留，未计为 PASS。

最终559个 build-source 字节、raw Git blob、CLI hash 与本地提交关联另存 proof。
设备、其它平台、SDK/feature/MSRV 范围按原证据复用或保持 NOT_RUN。
本项不覆盖 failure 表额外 key/route/backing 与 rollback、任意 Error/Arc/anyhow
私有分配、native runtime 存储和完整 freshness；128MiB仍非完整 manager heap/RSS
上限。L02 主项未关闭，账本23交付/6缺开发/L02待收口、有效30及 Q04 全部排除保持。
没有真实设备发现、配对、安装、恢复、push、发布或部署。

外部证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-seen-observed`。
原失败在父目录及 `final`，最终严格出口和 proof 在 `closeout`。
