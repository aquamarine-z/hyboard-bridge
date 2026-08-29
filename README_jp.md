<div align="center">
  <h1>hyboard-bridge</h1>
  <p><strong>X-board / V2board から Hysteria 2 コアへの認証・トラフィックブリッジ</strong></p>
  
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

## 概要 (Overview)

`hyboard-bridge` は、**X-board / V2board** パネルと **Hysteria 2 公式コア (v2.12+)** を接続するために設計された高性能なブリッジデーモンです。

ローカルで HTTP Webhook 認証インターフェースを公開し、定期的に `trafficStats` をポーリングすることで、公式 Hysteria 2 ノードに対してマイクロ秒レベルの認証応答と正確なトラフィック報告を提供します。

---

## 主な機能 (Features)

- **マイクロ秒のローカル認証**: `< 1ms` の応答時間を持つローカルの `POST /auth` Webhook 認証インターフェース。
- **オフラインディザスタリカバリ**: 認証ホワイトリストはメモリに保持されます。パネルが一時的にダウンしても、既存の接続や新しいハンドシェイクには影響しません。
- **永続的なトラフィック報告**: ネットワーク異常時にユーザートラフィックを自動的に蓄積し、復旧時に報告することで正確な課金を保証します。
- **マルチノード統合**: 単一の Hysteria 2 プロセスで複数のパネルやノードを同時に認証し、ホワイトリストを自動で集約します。
- **最小限のシステムオーバーヘッド**: Rust と Tokio ランタイムで構築されており、バイナリサイズは `< 15MB`、静的メモリ消費は `< 10MB` です。

*(詳細な設定とデプロイメントについては、英語の README を参照してください)*
