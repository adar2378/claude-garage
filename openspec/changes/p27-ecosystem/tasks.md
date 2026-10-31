# Tasks: p27-ecosystem

Rough; refined when the phase starts.

## 1. API v1

- [ ] 1.1 Export the API schema from the protocol crate
- [ ] 1.2 Version negotiation: manifest `api` + capabilities, actionable refusal message
- [ ] 1.3 API reference docs generated from the schema

## 2. SDKs

- [ ] 2.1 Rust SDK crate generated from the schema
- [ ] 2.2 TypeScript SDK package generated from the schema
- [ ] 2.3 Contract tests: both SDKs against a test daemon

## 3. Authoring tools

- [ ] 3.1 `claude-garage plugin new` (Rust, TypeScript, shell templates)
- [ ] 3.2 `claude-garage plugin dev`: link, reload on change, stream logs
- [ ] 3.3 Plugin-authoring `CLAUDE.md` template in the scaffold
- [ ] 3.4 Skill: write a plugin from a one-sentence description

## 4. Discovery and docs

- [ ] 4.1 `claude-garage plugin search` over the `claude-garage-plugin` topic
- [ ] 4.2 Docs: concepts, manifest reference, slots and actions, permissions, examples

## 5. Verification

- [ ] 5.1 A plugin created with `plugin new`, developed with `plugin dev`, published with the topic, found with `plugin search`, installed on a second machine
- [ ] 5.2 Record results in `verification.md`
