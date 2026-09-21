# Agora and Bellman Policy Optimization

## Agora

HAIRSPRING maps Agora's research DAG onto its existing append-only hash-chained log rather than adding a second authority store. `hs_world::research` records typed contributions with explicit `builds on` parents, artifacts, metrics, authors, verification targets, and verdicts. Its analysis view exposes leaders, neglected leaves, open hypotheses, unverified results, and contested targets. Evidence comes only from downstream work by another author. A verifier's newest verdict replaces that verifier's older verdict while history remains append-only.

Source audited: Yifan Zhang et al., "Agora: Git as Shared Memory for Collective AutoResearch," arXiv:2609.18094, including the paper's method and minimal contribution record. The public repository contains the paper site and figures, not the platform implementation, so claims that could not be checked against code were not copied as implementation facts.

## Bellman Policy Optimization

`hs_selfmod::bpo` implements the practical critic-free RLVR loss from "Bellman Policy Optimization," arXiv:2609.15987: group-normalized terminal-reward advantages and the smoothed complementary-probability mismatch weight `(1 + eps - mu) / (1 + eps - pi)`, with the paper's directional mask and cap. It intentionally does not replace HAIRSPRING's discrete held-out promotion gate. BPO is a gradient update for model weights and requires rollout/current token probabilities; pretending it directly optimizes prompt mutations would not be faithful.
