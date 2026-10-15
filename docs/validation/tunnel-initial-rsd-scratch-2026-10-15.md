# L02 initial RSD transport and parser scratch · 2026-10-15

初始探测现在沿用 manager reservation 的原 byte bank。kernel 和 userspace
activation 在 attach 取走 reservation 前捕获同一个 Arc；userspace runtime 和
后续 proxy RSD 共享该池。原生 handle 在探测前同步 attach，成功 complete、
失败 observation 和 caller cancellation 的原 native cleanup/join 责任保持。

banked 初始路径复用实际 H2 frame/demux/body、bootstrap wire、operation future
及 borrowed arena 的创建前准入。借用 arena 从原 header/body 两遍解析，不建立
完整拼接副本或 IndexMap/XpcValue 中间树。None-bank 仍走原解码和公开目录。
本项不修改已刷新目录使用的严格 checked parser；新增兼容 parser 专门保留
initial 的旧规则：空 UID、端口 0、缺失/错误 Services 成功为空、跳过非法 service
及 Port、Features 仅保留字符串且保留空项/重复项。原 semantic 错误仍逐条跳过，
六条耗尽才结束当前 mode；wire/decode 错误立即结束该 mode。两者仍使用新连接
执行 Active/Queued/Legacy/Passive 回退。新增池拒绝直接保留 ResourceLimit，
避免被下一连接的 EOF/拨号错误覆盖。

identity 仍在旧外层期限前解析；外层 30s、拨号 15s、bootstrap 3s 和响应
4/4/3/2s 原预算保持。后续 SDK 五次/500ms 重试和 historical rsd()/initial_rsd_probe()
没有改变。无 mdns 的 direct 仍为 NotAttempted/DirectProbeFeatureUnavailable，
即使传入关闭或空池；proxy 仍可用。

公开 RsdHandshake 的 String/HashMap/Vec 持久存储仍 MISSING。其公共字段、Clone
及 into_rsd() 允许目录独立于 ConnectedDevice 存活，设备侧费用 sidecar 无法跟随
原 buffer。此次没有用短寿命 sidecar、泄漏 permit、复制目录或改成 None 来假装
覆盖，也没有从 retained_metadata_bytes 或 published cap 减掉这部分。manager 原
后置 metadata 准入继续存在，其 HashMap backing 估算不算创建前实际存储合同。

新增 11 项 all-feature XPC 回归：legacy parser 差分 3 项、四 mode、真实 IPv6
localhost direct、同 mode semantic skip、六次耗尽后 fresh fallback、最后 parser
错误、wire 错误 fresh fallback、池拒绝不重连、body read 取消和严格 EOF。proxy
现有 fixture 同时运行 None/原池；activation fixture 拒绝 attach 后再查询 bank。
另有无 mdns 关闭池 direct skip 用例。仅 duplex/localhost，无设备操作。

第一轮 XPC201/activation5/proxy3 通过；随后无 mdns 测试编译暴露原 service
test 使用受其它 feature 控制的 XpcValue re-export，已改为直接 message 模块导入。
失败日志保留在 round1。独立只读审查还发现 fixture 在早退时可能等待未使用
client 的 peer；现先关闭 client，再有界 join，超时仅 abort/join 自有 peer。
第二轮冻结 603 个 build-source，源码无漂移；XPC201、activation5、proxy3、
无 mdns proxy/direct4、prepared4、existing18、core tunnel131 全部通过。完整
CLI1480/0FAIL/13 原 ignored 通过（不是此前仅 tunnel504 的过滤命令）。
strict core/CLI all-feature/all-target Clippy、build/fmt/diff 通过；无默认及无
tunnel raw feature 编译通过，实际 cfg warnings 保留。只读 manager1 通过，
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
受测 CLI SHA-256：
`3bf07b933ca6103629001cd71acdcad1dbdb6d373aa101e412da31b9ee0497fd`。
完整日志、受测 binary、source freeze、当前及提交后 raw Git blob proof 均在 round2。

证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-initial-rsd-scratch`。

128MiB 仍是已准入自有对象池，非完整 manager heap/RSS；公开目录、SDK 拷贝、
Bytes 私有 header、Tokio/private error/allocator 和原完整 freshness 等继续核查。
本子项不关闭 L02，23 交付/6 缺开发/L02 待全审计、有效分母 30 和 Q04 全部
排除保持。仅本地提交；其它平台/设备/SDK/MSRV 和未执行 feature 保持 NOT_RUN。
