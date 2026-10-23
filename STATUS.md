<!-- Generated; do not edit by hand. Source: .project/events.jsonl. Live view: npx slotledger web -->
# Project status

## Live decisions

| Category | Slot | Current value | Status | Source | Updated | Notes |
|---|---|---|---|---|---|---|
| Goal | goal/matrix-scan-reliability | 用户尽量保持稳定时，Matrix OCR 应在 6–7 秒内完成，扫描成功率达到 95%。 | decided | user | 2026-10-23 02:28 |  |
| Approach | approach/matrix-canonical-grid | 使用正文 2:1、单格 1.4:1 的固定版式，统一回正后机械切成 7×10 格，保留完整字符并等比例补白进入模型。 | decided | user | 2026-10-23 03:29 |  |
| Approach | approach/matrix-ocr-color-input | 在相同定位结果和模型权重下，对照亮度灰度、红色色差与相对纸张底色的 RGB 色差输入，依据视频实测选择 OCR 默认输入。 | decided | user | 2026-10-23 05:29 |  |
| Approach | approach/matrix-realtime-locator | 优先重写 Matrix CV 模块的定位、跟踪和几何验证逻辑，按快速框体跟踪与慢速精修识别划分职责；先通过现状核对与分阶段回放诊断明确错误来源，避免在现有检测器上持续叠加补丁。 | decided | user | 2026-10-23 05:58 |  |
| Approach | approach/matrix-session-validation | 建议以独立真实扫描验证 7 秒内整卡全对率，超时、定位失败、拒识和误识均进入分母，按卡片与拍摄会话隔离验证集，并分别报告错误放行率、耗时分布和成功率置信区间。 | accepted by default | AI | 2026-10-23 02:31 |  |
| Approach | approach/matrix-streaming-recognition | 实现有界生产者消费者管线和 Rust Core dispatcher，采集与微批 OCR 并发，按覆盖、冲突和复核需求调度，确认状态可撤销，扫描按会话启动计时。 | decided | user | 2026-10-23 03:29 |  |
| Approach | approach/testflight-existing-signing | 自动 TestFlight 发布直接使用仓库已有 IOS_CERTIFICATE、IOS_CERTIFICATE_PASSWORD 和 IOS_MOBILE_PROVISION，签名配置与现有证书和描述文件一致。 | decided | user | 2026-10-23 10:05 |  |
| Environment | env/matrix-iqrsh-replay | 视频处理与脱敏在本地完成，匿名字符裁图及指标上传 TSUBAME，直接用 iqrsh 执行测试，通过一个脚本生成每秒起点的独立 6 秒扫描窗口正确率折线图。 | decided | user | 2026-10-23 03:29 |  |
| Assumption | assum/matrix-card-size | 本轮测量卡片为用户确认的普通银行卡大小，采用 ID-1 标称尺寸 85.60×53.98 mm 作为外框度量基准。 | decided | user | 2026-10-23 03:18 |  |
| Assumption | assum/matrix-measured-body-ratio | 本轮同一卡片的本地测量支持正文宽高比采用约 2.00:1、单格约 1.40:1；正文不含顶部 A–J 和左侧 1–7 标签，尚未证明所有卡片版式通用。 | accepted by default | AI | 2026-10-23 03:18 |  |
| Assumption | assum/matrix-scan-success-definition | 本轮方案暂按从用户启动扫描起 7 秒内，整张卡 70 格全部正确且无需手动修改定义成功；超时、拒识和误识均计失败。 | accepted by default | AI | 2026-10-23 02:28 |  |

## Open questions

(none)

## Tasks

| Task | Status | Verification |
|---|---|---|
| 评估 7 秒内整卡成功率 95% 的 OCR 方案 | verified | ref: reported passing, 证据支持本轮方案评估：当前采集与识别串行且采集上限为 8 秒；已有裁切截断高置信误识和行边界偏移复现；同模型定位改动改善冻结视频表现。通过仅表示方案评估完成，产品的 7 秒内 95% 成功率尚未验证。 [crates/isct-core/src/matrix_scan/capture.rs (sha256 b178a9202606), .cache/reviews/pr18-review.json (sha256 4047c6aaf262), .cache/reviews/pr18-clipped-probe.log (sha256 f4d105d4fbc0), .cache/reviews/pr18-probes-isolated.log (sha256 b8eedd49d052), .cache/ocr-locator-retest-20261022/comparison-summary-local.json (sha256 e1e8c98933be), https://proceedings.mlr.press/v70/guo17a.html] (2026-10-23 02:31) |
| 核对最新 Matrix 定位回正与 Q 裁边原因 | verified | ref: reported passing, 已核对最新远端引用与生产源码，确认回正已有实现、主分支与 PR 定位路径不同。既有平台峰值测试适用于未变化的 PR 版本；本次仅完成实现分析，未复现用户特定 Q 失败帧、未修改 OCR。 [https://github.com/isct-companion/isct-companion/pull/18, https://github.com/isct-companion/isct-companion/blob/e9945d8e0719b7e4341b1a25e362ef6ef37962d1/crates/isct-ocr/src/engine/scan_image_locator.rs, crates/isct-ocr/src/engine/geometry.rs (sha256 9b99bebb8566), .cache/reviews/pr18-probes-isolated.log (sha256 b8eedd49d052)] (2026-10-23 02:41) |
| 评估并发批量 OCR 与规则网格切割方案 | verified | ref: reported passing, 代码支持复用批量模型并改造阶段限制，平面单应映射支持规则版式切格。本次通过仅表示可行性评估完成；15–30 fps、7 秒与 95% 均未通过真机验收。 [crates/isct-core/src/matrix_scan/capture.rs (sha256 b178a9202606), https://github.com/isct-companion/isct-companion/blob/e9945d8e0719b7e4341b1a25e362ef6ef37962d1/crates/isct-ocr/src/engine/alphabet.rs, https://docs.opencv.org/4.13.0/d9/dab/tutorial_homography.html] (2026-10-23 02:57) |
| 用本地视频与测试图测量 Matrix 版式比例并验证机械切格 | verified | run: `.cache/ocr-venv/bin/python .cache/ocr-ratio-study-20261023/check_local.py` passed (darwin 27.0.0 arm64, node v22.23.0, 2026-10-23 03:18) |
| 实现机械切格与并发调度并在 TSUBAME 回放 6 秒窗口 | verified | ref: reported passing, 机械切格、并发队列、独占微批调度和本地脱敏/i​​qrsh 回放已完成；Core 10 项、OCR 20 项、私有清晰帧 1 项、文本翻译 27 项通过，桌面编译和类型检查及静态检查通过。逐秒 6 秒窗口图已目视检查。54 个窗口整卡正确完成 3 个（5.56%），5 个窗口出现部分格子误确认；这是预热模型的混合耗时视频回放，95% 成功率和 15–30 fps 目标均未达成，实机验收项 OCR-CAMPUS-CARD 待测。 [.cache/matrix-streaming-20261023/results.json (sha256 70137b5ee833), .cache/matrix-streaming-20261023/iqrsh.log (sha256 421cb563170f), .cache/matrix-streaming-20261023/six-second-accuracy.png (sha256 c6cc8c67197c), .cache/matrix-streaming-20261023/validation.json (sha256 d09ae1f95b53), /Users/miaozean/.codex/worktrees/ca15/IsctApp/docs/verification/matrix-streaming-results.json (sha256 26e8e36c8a12); windows=54, successfulWindows=3, wholeCardSuccessRate=0.05555555555555555, windowsWithWrongConfirmedCells=5, captureMedianMs=318.593042, batchMedianMs=18.263364, target95PercentMet=0] (2026-10-23 04:17) |
| 构建包含机械切格与并发 dispatcher 的 1.1.7-dev 测试 App | verified | ref: reported passing, 更新后的 1.1.7-dev 已构建，CFBundleShortVersionString 为 1.1.7-dev，codesign --verify --deep --strict 返回 0。 [/Users/miaozean/Desktop/workspace/IsctApp/.cache/matrix-locator-20261023/build-verification.json (sha256 4026415a2b29)] (2026-10-23 05:41) |
| 优化实时框体定位并验证移动延迟与视频表现 | verified | ref: reported passing, 本地提交 96a97f3f：小图/ROI 与颜色条带精修已实现；119 帧和连续 300 帧完成固定几何输入对照。21 项 OCR 测试、clippy、无 OCR 后端编译、桌面构建通过。纸张色差整帧正确未提升且高置信度错误增多，默认仍为灰度。此验证覆盖实现与离线视频，不代表真实相机 OCR-CAMPUS-CARD 或 95% 会话成功率验收。 [/Users/miaozean/.codex/worktrees/ca15/IsctApp/docs/ocr-locator-study-20261023/README.md (sha256 bdad65b2493c), /Users/miaozean/.codex/worktrees/ca15/IsctApp/docs/ocr-locator-study-20261023/sampled.json (sha256 56c064adb14c), /Users/miaozean/.codex/worktrees/ca15/IsctApp/docs/ocr-locator-study-20261023/continuous.json (sha256 87f309ceb5ad); capture_p50_ms=51.390292, tests_passed=21, tests_ignored=4, continuous_luma_exact=55, continuous_paper_exact=54] (2026-10-23 05:41) |
| 核对 Matrix 实时管线现状并明确重写与诊断边界 | verified | ref: reported passing, 主代理与 6.1-sol/high 完成只读审查。确认已有独立 OCR 消费者、单帧 in-flight、限容限时候选和多帧投票；缺少独立快速跟踪、全网格联合 H 拟合及 snapshot 源帧/顺序门禁。最新实机记录77次capture/5631ms、10次限时内几何结果；计数起点和截止口径不同，不将10/77直接称为定位失败率。已明确先做分阶段同帧几何和源帧到显示延迟诊断，重写 CV 时保留 Core 生命周期、调度与证据契约。代码未修改，训练与算法重写尚未启动。 [/Users/miaozean/.codex/worktrees/ca15/IsctApp/apps/desktop/src/platform/matrix-camera.ts (sha256 e61e85ac85ec), /Users/miaozean/.codex/worktrees/ca15/IsctApp/crates/isct-core/src/matrix_scan/capture.rs (sha256 804a36128415), /Users/miaozean/.codex/worktrees/ca15/IsctApp/crates/isct-ocr/src/engine/scan_image_locator.rs (sha256 7242bdab7c88), /Users/miaozean/Library/Caches/app.isctcompanion/matrix-scan/latest.json (sha256 4ad808a836eb)] (2026-10-23 05:59) |
| 核对自动 TestFlight 发布流程 | verified | ref: reported passing, 完成静态审阅和线上只读核对：IOS_TESTFLIGHT_ENABLED=true；三个 APPLE_API_* 密钥已配置；可选 IOS_DISTRIBUTION_CERTIFICATE 两项缺失，旧 IOS_CERTIFICATE 等三项不被工作流读取。运行当时处于 Core XCFramework 构建阶段，签名上传未验证。 [https://github.com/isct-companion/isct-companion/blob/5f6982a4cc6afd33d70b64b64aad0b34682dd29a/docs/mobile-release.md, https://github.com/isct-companion/isct-companion/blob/5f6982a4cc6afd33d70b64b64aad0b34682dd29a/.github/workflows/ios-release.yml, https://github.com/isct-companion/isct-companion/actions/runs/39935374810] (2026-10-23 10:01) |
| 对齐已有 iOS 签名并发布 TestFlight | in_progress | not verified |

## Change history

- approach/matrix-streaming-recognition: v1 "建议先修复定位与裁切缺陷，再将串行采集后识别改为锁定网格、持续跟踪、边采边识别、重点补采不确定格和整卡终检，并由 Rust Core 统一管理从启动扫描计时的 7 秒预算。" → v2 "建议将摄像头流程实现为有界生产者消费者管线：最新帧定位或跟踪、每格候选池、Rust Core dispatcher 按覆盖与冲突调度微批 OCR，并持续更新可撤销的确认状态，统一遵守启动后 7 秒预算。" → v3 "实现有界生产者消费者管线和 Rust Core dispatcher，采集与微批 OCR 并发，按覆盖、冲突和复核需求调度，确认状态可撤销，扫描按会话启动计时。"
- approach/matrix-canonical-grid: v1 "建议用固定版式的正文比例和规则网格统一拟合单应变换，按标准坐标机械切出 7×10 格，保持字符比例并留白进入模型，取消逐行独立追逐黑灰边界。" → v2 "使用正文 2:1、单格 1.4:1 的固定版式，统一回正后机械切成 7×10 格，保留完整字符并等比例补白进入模型。"
- approach/matrix-realtime-locator: v1 "实时定位优先采用几何算法和现有图像库，分离低分辨率候选搜索与原图精修，并通过逐阶段计时及连续视频验证性能；必要时可引入 Rust OpenCV 或约 2 MB 的定位模型。" → v2 "优先验证小图找位置与连续帧缩小搜索范围，并在同一批视频帧上对照标准灰度、单通道、自定义灰度投影和颜色差异特征，依据速度、定位率与裁格精度选择几何算法；必要时可引入 Rust OpenCV 或约 2 MB 定位模型。" → v3 "采用小图粗定位与连续帧缩小搜索范围；重点对照第二阶段的亮度精修和颜色条带精修，以可配置色差规则在粗定位边界附近拟合四条边，颜色不足时回退亮度路径，原图保留用于机械切格和 OCR。" → v4 "优先重写 Matrix CV 模块的定位、跟踪和几何验证逻辑，按快速框体跟踪与慢速精修识别划分职责；先通过现状核对与分阶段回放诊断明确错误来源，避免在现有检测器上持续叠加补丁。"
