<div align="center">
  <h1>hyboard-bridge</h1>
  <p><strong>X-board / V2board 및 Hysteria 2 공식 코어를 위한 인증 및 트래픽 브릿지</strong></p>
  
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

## 개요 (Overview)

`hyboard-bridge`는 **X-board / V2board**와 **Hysteria 2 공식 코어 (v2.12+)**를 위해 설계된 고성능 브릿지 데몬입니다.

로컬 HTTP Webhook 인증 인터페이스를 열고 `trafficStats` API를 주기적으로 폴링함으로써, 공식 Hysteria 2 노드에 마이크로초 수준의 인증 응답과 정확한 트래픽 보고를 제공합니다. 복잡한 다중 노드 및 패널 통합 아키텍처를 기본적으로 지원합니다.

---

## 주요 기능 (Features)

- **마이크로초 로컬 인증**: `< 1ms` 응답 시간을 갖는 로컬 `POST /auth` Webhook 인증 인터페이스.
- **오프라인 재해 복구**: 인증 화이트리스트가 메모리에 상주합니다. 패널이 일시적으로 다운되더라도 기존 연결 및 새로운 핸드셰이크에 영향을 주지 않습니다.
- **영구적인 트래픽 보고**: 네트워크 이상 중에 사용자 트래픽을 자동으로 누적하고 복구 시 보고하여 정확한 과금을 보장합니다.
- **다중 노드 통합**: 단일 Hysteria 2 프로세스가 여러 패널과 노드를 동시에 인증하며 화이트리스트를 자동으로 통합합니다.
- **최소한의 리소스 오버헤드**: Rust와 Tokio 런타임으로 구축되어 바이너리 크기는 `< 15MB`이며 정적 메모리 소비는 `< 10MB`입니다.

*(자세한 설정 및 배포 방법은 영어 README를 참조하세요)*
