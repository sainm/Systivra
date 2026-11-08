//! Capability ゲートウェイ、Agent、Workflow、MCP。
//!
//! 境界（`02-アーキテクチャ` §3）：`runtime` は**業務アルゴリズムを持たない**。
//! 能力の名前・権限・予算・監査だけを持つ。

#![forbid(unsafe_code)]

pub mod capability;

pub use capability::{
    budget_exceeded, BudgetUsage, Capability, CapabilityError, CapabilityGateway, Principal,
};
