# L02 私有过滤器明文扩展头修复（2026-10-25）

本项修复 HAO、RH2/SRH、AH 可见明文的端点分类与分片关联，属于 L02 的有限可验收子项。L02 整体、其余六个主项和原账本口径不变；Q04 继续排除。

HAO 按真实 TLV 长度、对齐、唯一性和位置解释源地址；RH2/SRH 分开处理活动路由与 Segments Left 为零的状态，保留下一跳与可能最终端点的区别。该分类没有创建 Mobility binding 或 SID 配置。[RFC 6275](https://www.rfc-editor.org/rfc/rfc6275.html#section-6.3)、[RFC 8754](https://www.rfc-editor.org/rfc/rfc8754.html#section-4.3.1.1)。

AH 按其真实长度单位与 IPv6 对齐读取明文链，不执行 SA 验证。[RFC 4302](https://www.rfc-editor.org/rfc/rfc4302.html#section-2.2)。offset-zero 的 canonical TCP 身份独立于进入 Fragment 处理点时的关联 key；atomic headers 逻辑跳过，原 packet owner 不重建。60 秒期限取首次进入 ingest 的时刻，后续分片不续期。[RFC 8200](https://www.rfc-editor.org/rfc/rfc8200.html#section-4.5)。

仍使用原 FlowLease、真实保留源端口与原字节 bank；动态元数据增长先支付实际 backing，拒绝时保存 incoming 和 prior packet owner。原六个测试及其完整文件前缀逐字节不变，新增十个回归包含两方向有效 checksum、同指针转发、首片与 key 差异、组合扩展头、拒绝/退款及未完成边界的实证。

| 主控门禁 | 结果 |
| --- | --- |
| Windows Core userspace-only 过滤器 | 16 PASS |
| Core all-features/all-targets Clippy（Windows，-D warnings） | PASS |
| WSL Linux userspace-only 编译 | PASS，27 条最小特性 unused warnings |
| 直接执行该次生成的 Linux ELF，仅过滤器组 | 16 PASS |
| 两个过滤器源文件 rustfmt --check | PASS |
| git diff --check | PASS |

778 份源码在串行门禁期间冻结且 SHA 无漂移。证据位于外部 `l02-filter-plaintext/round1/` 的六份 result/log、build-sources.json 与 source-verification.json。Linux Cargo 输出未提供测试汇总，因此额外直接执行刚生成的 ELF，依据其 16/0 汇总确认执行。全部 Cargo/build/test 由主控执行；只有真实本机 socket reservation，没有接口、地址、路由或设备操作。

本次仅提交过滤器两个源文件与本验证/索引文档。并行 CLI prepared pipeline、exact replacement proof 和 Unix actor 的未提交改动仍属 WIP，不由本报告验收。本项未重建实际 CLI 可执行文件，未复跑完整 CLI/host/manager，也不将既有二进制归属于当前 WIP。

ESP、IP-in-IP、未知原生转换仍转发原 owner；新增测试明确展示内层私有 TCP tuple 可随外层 IP-in-IP 转发。这不是完整隔离保证。原生 SA/decapsulation/SID/binding 约束、嵌套普通分片、Unix actor、精确替换与全部 retainer 审计继续，不能把此项等同于 L02 完成。
