# Tasks: p27-ecosystem

Rough; refined when the phase starts.

## 1. API v1

- [x] 1.1 Export the API schema from the protocol crate (`schemas/garage-plugin-api.schema.json`, `garage_protocol::api`)
- [ ] 1.2 Version negotiation: manifest `api` + capabilities, actionable refusal message
- [ ] 1.3 API reference docs generated from the schema

## 2. SDKs

- [x] 2.1 Rust SDK crate generated from the schema (hand-written over the protocol types, publishable; see design part A3)
- [x] 2.2 TypeScript SDK package generated from the schema (`sdk/ts`; types generated, client hand-written)
- [ ] 2.3 Contract tests: both SDKs against a test daemon

## 3. Authoring tools

- [x] 3.1 `claude-garage plugin new` (Rust, TypeScript, shell templates)
- [x] 3.2 `claude-garage plugin dev`: link, reload on change, stream logs
- [x] 3.3 Plugin-authoring `CLAUDE.md` template in the scaffold
- [x] 3.4 Skill: write a plugin from a one-sentence description

## 4. Discovery and docs

- [x] 4.1 `claude-garage plugin search` over the `claude-garage-plugin` topic
- [x] 4.2 Docs: concepts, manifest reference, slots and actions, permissions, examples (`docs/plugins/`)

## 4b. Example plugin: OpenSpec progress

- [x] 4b.1 `openspec-progress` plugin, built only with the public SDK and `plugin new` (proves the authoring path end to end)
- [x] 4b.2 Reads `openspec/changes/*/tasks.md` in the focused workspace's folder and counts `- [x]` vs `- [ ]` per change; watches the files and updates live
- [x] 4b.3 Shows a small per-workspace progress widget (rail header slot) and an overlay listing each active change with its done/total and percentage
- [ ] 4b.4 Published as a separate repo with the discovery topic, as the reference plugin in the docs

## 5. Verification

- [ ] 5.1 A plugin created with `plugin new`, developed with `plugin dev`, published with the topic, found with `plugin search`, installed on a second machine
- [ ] 5.2 Record results in `verification.md`
