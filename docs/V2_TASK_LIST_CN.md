# Cow Boy V2 任务列表

最后更新：2026-02-09

状态图例：
- [ ] 待办
- [-] 进行中
- [x] 已完成

## 1) 规划与契约
- [x] 确定 V2 架构和服务职责
- [x] 确定所有服务的 API 契约
- [x] 确定 Kafka 命令/步骤消息契约
- [x] 确定 DynamoDB 表设计
- [ ] 在编码前审查 V2 设计文档的最终一致性

## 2) 前端（保留当前设计 + 新增状态界面）
- [x] 添加游戏状态栏（状态、游戏 ID、回合、轮次、当前玩家）
- [-] 添加回合 + 超时面板（倒计时 + 进度条 + 超时闪烁）
- [x] 将开始按钮接入 V2 start-game API
- [x] 将命令提交接入 Web Service API
- [x] 添加快照引导和 WebSocket 同步流程
- [x] 添加重连/重新同步行为
- [x] 根据 WebSocket 和快照状态显示胜利消息/动画

## 3) 后端 - Game Manager Service（Rust）
- [x] 搭建 Rust 服务项目结构
- [x] 实现创建游戏 API（使用自定义地图）
- [x] 实现创建游戏 API（无地图 -> 默认地图流程）
- [-] 实现默认地图生成和持久化
- [x] 实现启动游戏 API
- [-] 发出游戏启动事件用于 watcher 广播路径
- [x] 添加内部 finish 端点用于 game-service 控制的结束状态转换
- [x] 添加 AWS Lambda 运行时兼容性（`aws-lambda-rust-runtime`）

## 4) 后端 - Web Service（Rust）
- [x] 搭建 Rust 服务项目结构
- [x] 实现提交命令 API
- [-] 验证请求格式并发布到 Kafka
- [ ] 添加 command_id 的幂等性处理（如果最终契约要求）
- [x] 添加 AWS Lambda 运行时兼容性（`aws-lambda-rust-runtime`）

## 5) 后端 - Game Service（Rust）
- [x] 搭建 Rust 服务项目结构
- [x] 添加 HTTP 命令处理端点用于当前本地集成
- [x] 当仅剩一名存活玩家时将游戏标记为 `FINISHED`
- [ ] 从 Kafka 消费命令
- [ ] 加载当前状态并验证命令
- [ ] 应用游戏规则并产生状态转换
- [ ] 每个消费的命令在 DynamoDB 中持久化一条步骤记录
- [ ] 将步骤事件发布到 Kafka
- [ ] 处理超时和忽略超时的命令行为

## 6) 后端 - Timer Service（Rust）
- [x] 搭建 Rust 服务项目结构
- [ ] 基于已应用的步骤事件启动/重置每回合计时器
- [ ] 计时器到期时发布超时命令
- [ ] 确保回合切换时取消旧计时器

## 7) 后端 - Game Watcher Service（Rust）
- [x] 搭建 Rust 服务项目结构
- [x] 实现最新快照 API
- [x] 实现 WebSocket 流 API（`from_turn_no`）
- [-] 向客户端流式传输有效的已应用事件
- [x] 广播游戏启动生命周期事件
- [x] 广播游戏结束生命周期事件
- [x] 添加 AWS Lambda 运行时兼容性（`aws-lambda-rust-runtime`）

## 8) 基础设施和存储
- [-] 创建 DynamoDB 表：`default_maps`
- [-] 创建 DynamoDB 表：`game_instances`
- [-] 创建 DynamoDB 表：`game_steps`
- [ ] 添加所需索引（用于命令去重/查询）
- [-] 配置 Kafka topics 和保留策略

## 9) 集成和测试
- [x] 为共享模型和 API 验证路径添加初步单元测试
- [x] 创建用于手动 + API 验证的综合测试用例文档
- [x] 端到端测试：创建 -> 启动 -> 进行回合
- [x] 端到端测试：结束状态转换 + WebSocket `GAME_FINISHED` 事件
- [ ] 端到端测试：超时流程和忽略的迟到命令
- [ ] 端到端测试：watcher 快照 + 流同步
- [ ] 端到端测试：从 `from_turn_no` 重连
- [ ] 命令吞吐量的负载/性能健全性检查

## 10) 交付
- [x] 更新本地运行/部署文档
- [x] 添加 docker-compose 技术栈用于一键本地启动
- [x] 添加 Makefile 快捷命令用于常用设置/运行操作
- [ ] 准备 V2 发布检查清单
- [ ] 使用前端进行最终验收测试

## 任务更新策略
- 当任务状态变更时更新此文件。
- 任务开始时从 `[ ]` 改为 `[-]`。
- 任务完成时从 `[-]` 改为 `[x]`。
- 范围变更时立即添加新任务。
