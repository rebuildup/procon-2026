---
name: worktree-workflow
description: 1つのgit working treeを共有mutable workspaceとして並列実装せず、Worktrunk等で各workerへ独立したworktree-workflow状態を作る時に使用する。
---

# Worktree Workflow

git working treeは複数agentで共有するmutable workspaceではない。

**1 worker = 1 isolated mutable runtime** を提供するために、git worktree、またはWorktrunk等のworktree-workflow toolでbranch / working tree / port / process / database stateを分離する。

## When to use

次の場合に使用する。

- 複数agent / worker / subagentで並列実装する
- 同一branchを複数agentが共有しないruntimeを確立したい
- workerごとに独立したbuild / test / runtime / port / database stateを割り当てる
- Worktrunk (worktree workflow CLI) を導入・運用する
- 既存worktreeのlifecycle管理 (作成 / 同期 / 削除 / archive) を行う
- main working treeをdurable integration stateとして保護したい

次だけを理由に起動しない。

- 単一agent / serial実装
- 1workerしか起動しないtrivialな変更
- worktree作成がCI runner内に閉じるCI workflow

## Core principle

> **git working tree is NOT an isolation boundary.**

branchを分けただけで実装が独立するとは限らない。

分離しなければならない対象:

- working tree (filesystem state)
- branch (git state)
- build artifact cache
- node_modules / cargo target / .cache等
- port / socket
- process
- database / local state file
- env / secret / config
- runtime service

worktree-workflowはこれらを1unitで分離する。

## Why Worktrunk

Worktrunkはworktree lifecycle CLIであり、次を提供する。

- branchに対応したworktreeを1コマンドで作成
- worktreeから独立したbuild / test / runtime
- 古いworktreeの掃除
- branch / commit / status統合表示

Worktrunkを使う場合、projectのcommand policyを明示する。

例:

```text
wt switch <branch>            # worktree切替
wt switch -c <branch>         # worktree新規作成
wt list                       # worktree一覧
wt remove <worktree>          # 削除
wt config worktree-path ...   # worktree path policy
wt config port-allocator ...  # port isolation policy
```

`wt` は例。project commandに合わせる。

## Setup

### 1. Install Worktrunk

project管理:

```text
mise use --global wrktrunk@latest
# or
brew install wrktrunk
# or
cargo install worktrunk
```

### 2. Project configuration

`~/.config/wrktrunk/config.toml` またはproject local configにworktree path / branch naming / port allocator等を設定する。

最低限:

- worktree path pattern
- base branch (release-x-y-z / main)
- worktree auto-clean policy
- port isolation policy
- env / secret forwarding policy

### 3. Verify isolation

最低限次を確認:

- 別worktreeで同一branch nameを生成できないか
- 別worktreeで同一portにbindしてconflictしないか
- 別worktreeで共有cache directoryが衝突しないか
- 別worktreeで共有database / file lockが衝突しないか
- main working treeに他workerの未commit変更が漏れていないか

## Daily workflow

### Worker spawn

```text
1. ticket branch名を確定
2. wt switch -c <issue-number>
3. 独立したworking treeでinitial commit作成
4. 独立したport / cache / dbを割り当て
5. remote publish + Draft PR作成 (github-delivery参照)
6. worker / subagentをこのworktreeにpin
```

### Worker teardown

```text
1. PR merged / Issue closed / superseded
2. wt remove <worktree>
3. local cache / port / db cleanup
4. remote branch archive / delete (project policy)
```

## Isolation patterns

### Port isolation

workerがlocal API / server / database / queueを必要とする場合:

- `hash_port(branch_name)` で決定的portを生成
- 同一portに複数workerがbindしないようallocation記録
- 必要ならport registry fileをproject root配下 `.tmp/` へ置く

### Cache isolation

- `target/` (Rust) はworktree単位のworking tree内へ配置される
- `node_modules/` はworktree単位
- `.cache/` はworktree単位
- 共有cacheが必要な場合は明示的に分離keyで管理

Rust/Cargo capacityのoperational defaultsは project-init ADR-0024 に従う。

**不変条件 (ADR-0024 §1)**

- Cargo `target/` / `build.build-dir` / incremental stateはworktree-local mutable output
- 共有 `CARGO_TARGET_DIR`・`target/` symlink・複数worktreeで同一のbuild-dirを禁止
- Cargoのtarget-dir lockingは並列buildを直列化し得る。divergent worktree間の
  fingerprint/artifact correctnessはcurrent Cargoが保証しない (cargo#16804 / cargo#17312)

**共有してよいcacheable層 (ADR-0024 §2)**

- CARGO_HOME registry/git download cache (defaultで既に共有)
- read-only toolchain cache
- bounded sccache — cross-worktreeのRust hitは計測必須 (SCCACHE_BASEDIRS:
  mozilla/sccache#2652 未解決)。未検証はfuture-facing。計測は Issue #218

**seedingはhost gate付き (ADR-0024 §3)**

- probe: 同一fs上で `cp --reflink=always /tmp/.p /tmp/.p2` (`Operation not supported` なら非reflink)、またはfs type確認
- reflink対応 (APFS / btrfs / XFS / ReFS) → warmなcompatible base worktreeから
  `wt step copy-ignored --require-include` で `.worktreeinclude` allowlist分だけ
  `target/` をCoW seed。Worktrunkが `(reflinked, no extra disk)` を報告するまで信用しない
- 非reflink (本host: ext4、probe失敗) → `.config/wt.toml` の
  `[step.copy-ignored] exclude = ["target/"]` を維持 (full copyは容量を悪化させる)
- どちらの場合も symlink / 単一target-dir共有への置換は禁止
- `.worktreeinclude` はsource worktree (`--from`、default = primary) から読まれる
- seed後もCargoのfingerprintを通す。copied artifactを現在のsourceに対するvalidation evidenceにしない

**spawn contract (disposable agent worktree)**

- workerのcargo実行は `CARGO_INCREMENTAL=0` でprefix (短寿命 → incremental disk増幅が支配的。
  sccache採用時も必須)
- 人間のlong-lived primary checkoutをcargo defaultから機械的に外さない
- project-wide profile設定 (`[profile.dev]` 等) は Issue #218 の計測なしにflipしない

**selective reclamation (ADR-0024 §5)**

- build/testの前後に `cargo clean` を儀式的に挿入しない。`wt remove` 直前にも不要
  (`target/` はworktreeごと消える)
- disk pressure時はまず `cargo clean --dry-run -v` / `--doc` / `--release` / `-p <pkg>` 等の狭いstep

**evidence signal (ADR-0024 Evidence)**

- worktree / target disk footprint、clean build time、incremental rebuild time、
  sccache hit/miss、debug/test挙動、CI/local parity — tuning時は記録する

### Database isolation

- 各workerに独立したlocal DB instanceを割り当てる
- 同一DB instanceを複数workerで共有しない
- migrationは各workerで独立に適用

### Process isolation

- 各workerで起動するservice / daemonをworktree内にbind
- pidfileをworktree単位に配置
- 並列実行時もprocess treeを独立に管理

## Anti-patterns

禁止:

- 単一working treeで複数agentが同時commit / push
- 1つのbranchを複数worktreeで同時checkout
- port / db / cacheを共有mutable workspaceとして扱う
- worktree作成後にmain working treeへ戻り、実装を続ける
- Worktrunk等を入れず、手動git worktreeで運用する (project policy明示がない場合)
- worktree lifecycle管理なしでworktreeを累積
- 古いcompleted ticketのworktreeを残し続ける
- worktree内の未commit変更を捨てて良いと決めつける
- 同一workerが複数ticket branchのworktreeを同時に保持する

## Integration with other Skills

### parallel-orchestration

worker起動時にworktree-workflowでisolationを確立する。subagent / workerはworktree内にpinされ、独立したmutable runtimeを持つ。

### sandbox-runtime

sandbox-runtimeのisolation policyと整合する。worktree-workflowはfilesystem / git isolationを担当し、sandbox-runtimeはcontainer / VM / process isolationを担当する。

### github-delivery

ticket branch作成 -> first commit -> remote publish -> remote head SHA一致 -> Draft PRはworktree内で実行する。worktree作成直後に開始する。

### agent-recovery

interrupted / superseded workerのworktree cleanupはrecovery procedureに含む。checkpointはworktree path / branch / commit SHAを記録する。

### quality-gate

worker gate / integration gateはworktree単位で実行する。worktree isolationが崩れている場合はgate failureと同等のstale扱い。

## Goal

**1 worker / 1 subagentが独立したworktreeと独立したruntime stateを持ち、shared mutable workspaceで並列実装することを禁止する。Worktrunk等のworktree-workflow toolでlifecycleを明示管理し、main working treeをdurable integration stateとして保護する。**