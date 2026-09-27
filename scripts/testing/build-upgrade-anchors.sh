#!/usr/bin/env bash
# 按锚点 rev 构建并缓存旧版连接测试宿主，供升级兼容矩阵与连接恢复旧版互通复用。
#
# 用法：
#   bash scripts/testing/build-upgrade-anchors.sh [--all | 锚点…]   # 例如 a13 或 a07 a13
#   bash scripts/testing/build-upgrade-anchors.sh --clean             # 删除全部锚点缓存与构建目录
#
# 每个锚点取该 rev 的源码快照，覆盖当前 tests/hosts/connectivity，按需补上工作区成员声明并应用
# tests/upgrade-matrix/anchors/<锚点>.patch（仅在按能力分层仍无法覆盖时存在），然后以
# --no-default-features 加 tests/upgrade-matrix/host-features.json 中该锚点的能力 feature 构建。
# 补丁只允许改宿主与工作区成员声明。产物位于 <target>/upgrade-anchors/<rev>/bin/uc-connectivity-host，host.json 记录
# 缓存键、补丁摘要与构建耗时；缓存键不变时直接复用。构建默认用满全部逻辑核并关闭增量编译，
# 可用 CARGO_BUILD_JOBS 限制并行度。
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
anchors_json=tests/upgrade-matrix/anchors.json
patch_dir=tests/upgrade-matrix/anchors
features_json=tests/upgrade-matrix/host-features.json

target_directory() {
  cargo metadata --locked --no-deps --format-version 1 "$@" |
    node -e 'let s="";process.stdin.on("data",x=>s+=x).on("end",()=>process.stdout.write(JSON.parse(s).target_directory))'
}

anchor_field() {
  node -e '
    const [path, id, field] = process.argv.slice(1)
    const anchor = JSON.parse(require("fs").readFileSync(path, "utf8")).anchors.find(a => a.id === id)
    if (!anchor) process.exit(3)
    process.stdout.write(field === "id" ? anchor.id : anchor[field])
  ' "$anchors_json" "$1" "$2"
}

host_features() {
  # 未登记的新锚点沿用最后一个登记锚点的能力集合。
  node -e '
    const table = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).anchors
    const ids = Object.keys(table).sort()
    process.stdout.write((table[process.argv[2]] ?? table[ids[ids.length - 1]]).join(","))
  ' "$features_json" "$1"
}

all_anchor_ids() {
  node -e 'for (const a of JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).anchors) console.log(a.id)' "$anchors_json"
}

sha256() { shasum -a 256 | cut -d' ' -f1; }

host_source_digest() {
  # 宿主源码与该锚点补丁共同决定产物；目录内容按路径排序后摘要，避免 mtime 影响缓存。
  local id=$1
  {
    git ls-files -co --exclude-standard -- tests/hosts/connectivity | LC_ALL=C sort | while read -r file; do
      printf '%s\0' "$file"
      shasum -a 256 <"$file"
    done
    if [[ -f "$patch_dir/$id.patch" ]]; then shasum -a 256 <"$patch_dir/$id.patch"; fi
    printf 'features:%s\n' "$(host_features "$id")"
    printf 'script:'
    shasum -a 256 <"$0"
  } | sha256
}

root="$(target_directory)/upgrade-anchors"

if [[ "${1:-}" == --clean ]]; then
  [[ -d "$root" ]] || exit 0
  for source in "$root"/*/src; do
    [[ -f "$source/Cargo.toml" ]] || continue
    build=$(target_directory --manifest-path "$source/Cargo.toml" 2>/dev/null || true)
    if [[ -n "$build" && "$build" != "$root"* && -d "$build" ]]; then rm -rf -- "$build"; fi
  done
  rm -rf -- "$root"
  printf 'removed %s\n' "$root"
  exit 0
fi

ids=()
if [[ "${1:-}" == --all || $# -eq 0 ]]; then
  while read -r id; do ids+=("$id"); done < <(all_anchor_ids)
else
  ids=("$@")
fi

mkdir -p "$root"
status=0
for id in "${ids[@]}"; do
  rev=$(anchor_field "$id" engine_rev) || { printf 'unknown anchor: %s\n' "$id" >&2; exit 2; }
  out="$root/$rev"
  binary="$out/bin/uc-connectivity-host"
  key=$(printf '%s\n%s\n' "$rev" "$(host_source_digest "$id")" | sha256)
  if [[ -x "$binary" && -f "$out/host.json" ]] && grep -q "\"cache_key\": \"$key\"" "$out/host.json"; then
    printf '%s %s: cached\n' "$id" "${rev:0:12}"
    continue
  fi
  started=$(date +%s)
  source="$out/src"
  rm -rf -- "$source" "$out/bin" "$out/host.json"
  mkdir -p "$source" "$out/bin"
  if ! git cat-file -e "$rev^{commit}" 2>/dev/null; then
    git fetch --no-tags --quiet origin "$rev"
  fi
  git archive "$rev" | tar -x -C "$source"
  rm -rf -- "$source/tests/hosts/connectivity"
  cp -R tests/hosts/connectivity "$source/tests/hosts/connectivity"
  rm -rf -- "$source/tests/hosts/connectivity/target"
  member_added=false
  if ! grep -q '"tests/hosts/connectivity"' "$source/Cargo.toml"; then
    node -e '
      const fs = require("fs"); const path = process.argv[1]
      const text = fs.readFileSync(path, "utf8")
      const next = text.replace(/^members = \[\n/m, match => `${match}  "tests/hosts/connectivity",\n`)
      if (next === text) process.exit(4)
      fs.writeFileSync(path, next)
    ' "$source/Cargo.toml"
    member_added=true
  fi
  patch_sha=none
  if [[ -f "$patch_dir/$id.patch" ]]; then
    git -C "$source" apply "$repo/$patch_dir/$id.patch"
    patch_sha=$(sha256 <"$patch_dir/$id.patch")
  fi
  features=$(host_features "$id")
  lock_before=$(sha256 <"$source/Cargo.lock")
  log="$out/build.log"
  build_status=0
  (
    cd "$source"
    cargo fetch --quiet
    # 旧版宿主来自锚点 rev 的源码快照并覆盖了当前宿主，如实记录构建来源，不沿用调用方为当前树设置的值。
    # 快照自带的 .cargo/config.toml（A06 起为 jobs = 2）只为本地开发限峰，不决定锚点构建的并行度；调用方
    # 显式设置的 CARGO_BUILD_JOBS 仍然优先。快照构建目录用完即删，增量编译没有复用价值，关闭后各 rev 的
    # 工作区 crate 才能命中共享编译缓存，只改宿主时不必整棵重编。
    UC_HOST_ANCHOR="$id" UC_HOST_ENGINE_REV="$rev" \
      UC_ENGINE_SOURCE_COMMIT="$rev" UC_ENGINE_SOURCE_STATE=modified \
      CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-default}" CARGO_INCREMENTAL=0 \
      cargo build --offline -p uc-connectivity-host --no-default-features --features "$features"
  ) >"$log" 2>&1 || build_status=$?
  build=$(target_directory --manifest-path "$source/Cargo.toml")
  elapsed=$(($(date +%s) - started))
  if ((build_status != 0)); then
    printf '%s %s: build failed after %ss, see %s\n' "$id" "${rev:0:12}" "$elapsed" "$log" >&2
    status=1
    continue
  fi
  cp "$build/debug/uc-connectivity-host" "$binary"
  lock_after=$(sha256 <"$source/Cargo.lock")
  # 旧版构建目录体积大且可由共享编译缓存快速重建，产物复制后即回收。
  if [[ "$build" != "$root"* ]]; then rm -rf -- "$build"; else rm -rf -- "$source/target"; fi
  cat >"$out/host.json" <<JSON
{
  "anchor": "$id",
  "engine_rev": "$rev",
  "cache_key": "$key",
  "workspace_member_added": $member_added,
  "patch_sha256": "$patch_sha",
  "features": "$features",
  "lock_changed": $([[ "$lock_before" == "$lock_after" ]] && echo false || echo true),
  "build_seconds": $elapsed,
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
  printf '%s %s: built in %ss\n' "$id" "${rev:0:12}" "$elapsed"
done
exit "$status"
