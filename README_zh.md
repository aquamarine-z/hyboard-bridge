<div align="center">
  <h1>hyboard-bridge</h1>
  <p><strong>X-board / V2board 到 Hysteria 2 官方内核桥接程序</strong></p>
  
  <p>
    <a href="README.md">English</a> | 
    <a href="README_zh.md">简体中文</a> | 
    <a href="README_jp.md">日本語</a> | 
    <a href="README_kr.md">한국어</a>
  </p>

  <p>
    <a href="https://github.com/cedar2025/Xboard"><img src="https://img.shields.io/badge/X--board-Panel-blue?logo=github" alt="X-board"></a>
    <a href="https://github.com/v2board/v2board"><img src="https://img.shields.io/badge/V2board-Panel-blue?logo=github" alt="V2board"></a>
    <a href="https://github.com/apernet/hysteria"><img src="https://img.shields.io/badge/Hysteria_2-Core-green?logo=github" alt="Hysteria"></a>
  </p>

  <p>
    <img src="https://img.shields.io/badge/Rust-2024%20Edition-orange?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/Tokio-Async%20Runtime-blue?logo=tokio" alt="Tokio">
    <img src="https://img.shields.io/badge/Axum-HTTP%20Webhook-purple" alt="Axum">
    <img src="https://img.shields.io/badge/Docker-Image%20%3C%2015MB-blue?logo=docker" alt="Docker">
    <img src="https://img.shields.io/badge/License-MIT-yellow" alt="License">
  </p>
</div>

---

## 概述 (Overview)

`hyboard-bridge` 是一款专为 **X-board / V2board** 与 **Hysteria 2 官方内核 (v2.12+)** 设计的高性能桥接守护程序。

通过在服务器本地开放 HTTP Webhook 鉴权接口并定期轮询 `trafficStats` 接口，它为官方 Hysteria 2 节点提供微秒级的鉴权响应与精准的流量上报，支持复杂多节点与跨面板聚合架构。

---

## 核心特性 (Features)

- **微秒级本地鉴权**：在本地提供 `POST /auth` Webhook 鉴权，响应时间 < 1ms。
- **离线容灾机制**：鉴权白名单驻留内存，面板短暂离线不影响现存用户连接与新连接握手。
- **持久化流量上报**：网络异常期间自动累积用户流量，连接恢复后自动补报，确保计费精准。
- **多节点聚合支持**：支持单一 Hysteria 2 进程同时为多个面板、多个节点提供鉴权，白名单自动聚合。
- **极低资源开销**：基于 Rust 与 Tokio 运行时，二进制包大小 < 15MB，静态内存占用 < 10MB。

*(以下章节见英文 README)*
