# Security Audit: Creem Payment Flow

**Date:** 2026-04-22
**Auditor:** Agent Cascade
**Scope:** Bridge backend — Creem checkout, webhooks, callbacks, HMAC validation, subscription lifecycle, and reconciliation.

---

## Executive Summary

The Creem flow in Bridge has **strong foundational security** (HMAC verification, idempotency, RLS, constant-time comparison) but contains **two medium-risk gaps** and **several low-risk hardening opportunities**. No critical vulnerabilities were found.

---

## Findings

### 🔶 Metadata-based user resolution is last-resort and trust-dependent

**Location:** `c:/share/tyde/bridge/src/webhooks/processor.rs:419-422`

For Creem webhooks, `external_user_id` is ultimately resolved from provider metadata (`/metadata/user_id`, `/object/metadata/user_id`, etc.) after all DB lookups fail. This means **if an attacker can forge a signed webhook with tampered metadata, they can potentially reassign subscription events to a different user**. The HMAC signature prevents this *if* the secret is uncompromised, but the trust boundary is worth noting.

The "Creem orphan guard" at line 424-431 correctly suppresses the webhook if metadata resolution fails, which limits blast radius.

**Recommendation:** Consider adding a `verify_metadata_signature` or checksum if Creem supports signing nested metadata separately.

