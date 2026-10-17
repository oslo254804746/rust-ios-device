# L02 manual-success Preflight body owner · 2026-10-17

原 Preflight 的 body permit 位于 Box payload 内，unused/cancel Drop 不能
证明原 allocation 已释放才退款。本项接入 manual_success::Owner，先按精确
Layout 准入并分配实际空 Preflight 目的地，再创建原 WriteBatch/completed
存储；同一指针 place 后由 Box 外 permit 持费，Box 在前、body grant 在后。
没有多建 Box、临时补费或改变子对象的原预付顺序。

扫描的原 Owner 参数与 direct fixture 都已接入。成功 flush 使用原 into_inner
显式释放空 Box backing 后归还 body 费用，原 batch/completed 字段继续移给
原 flush future，各自存储费用保持；没有复制 tuple、重建 key 或提前释放
pending 记录。原 whole deadline、authority、失败版本、pending tuple 清理和
writer 顺序不改。预算 helper 使用实际新 Preflight 大小，原 Batch/backing
的创建前费用保持，不用旧 Box 指针宽度估计。

新增 3 项专项通过：原 Preflight 是首个精确已付 allocation，同指针填充且
在零余量下持有；into_inner 不再分配，先只归还 body，batch/completed 保留
费用至原字段结束；body 差一字节拒绝零分配，后续批次拒绝准确销毁原空
目的地；pending fixture 取消时原 Box 和子存储准确释放。原 consuming fixture
改用显式 into_inner，原 tuple/version/authority/超时断言保持。

首轮专项 3 PASS，有一个新测试的未使用 import 警告，进程结束后移除该 import。
最终轮完整测试包含这3项，tunnel 710 PASS / 0 FAIL / 2 原 ignored，upstream
access 10 PASS；all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、
实际只读 manager 1 项通过。没有把首轮专项冒充最终源码单独重跑。原配额、
writer 失败、whole deadline、原 IO、native handle/实际 joins/严格 EOF 均保留。

Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
659 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`308b9dfdeb7f40e54a8cb5602e9fb1bea42bd742c3b0576673487f86f566f448`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-preflight-body\round2`。

WriteBatch/scan/Freshness/Publication 共享 owner、私有 header 精确字节、opaque
error/IO、发现/SDK 原输出、公开初始 RsdHandshake 和完整 freshness/刷新/复用/
重启/manual epoch 审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；
L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。
仅本地提交，无真实设备、push 或发布。
