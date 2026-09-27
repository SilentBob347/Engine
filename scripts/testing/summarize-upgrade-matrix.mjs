#!/usr/bin/env node
// 汇总升级兼容矩阵各单元的 cell.json，输出 matrix.json 与 matrix.md（行 = 起始版本，列 = 目标版本，按维度分表）。
//
// 用法：node scripts/testing/summarize-upgrade-matrix.mjs <单元工件根目录…> [--partial]
//
// 可传多个根目录（例如 CI 各分片下载后的目录），按单元合并。
//
// 未执行的单元记为“未运行”；完整模式下存在未运行或与登记不一致的单元时以非零退出。

import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import { expandCells } from './resolve-desktop-anchors.mjs'

const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const roots = process.argv.slice(2).filter(argument => !argument.startsWith('--'))
const root = roots[0]
const partial = process.argv.includes('--partial')
if (!root) {
  process.stderr.write('usage: summarize-upgrade-matrix.mjs <artifact root...> [--partial]\n')
  process.exit(2)
}

const anchors = JSON.parse(readFileSync(join(REPOSITORY_ROOT, 'tests/upgrade-matrix/anchors.json'), 'utf8')).anchors
const expectations = JSON.parse(readFileSync(join(REPOSITORY_ROOT, 'tests/upgrade-matrix/expectations.json'), 'utf8'))
const points = [...anchors.map(anchor => anchor.id), 'head']
const registered = new Map(expectations.cells.map(cell => [cell.cell, cell]))
const expanded = expandCells(anchors.map(anchor => anchor.id))

function cellDirectories(directory, depth = 0) {
  if (!existsSync(directory) || depth > 3) return []
  if (existsSync(join(directory, 'cell.json'))) return [directory]
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isDirectory())
    .flatMap(entry => cellDirectories(join(directory, entry.name), depth + 1))
}

function readCells() {
  const cells = new Map()
  for (const directory of roots.flatMap(root => cellDirectories(root))) {
    const entry = { name: basename(directory) }
    const cell = JSON.parse(readFileSync(join(directory, 'cell.json'), 'utf8'))
    const result = JSON.parse(readFileSync(join(directory, 'result.json'), 'utf8'))
    // 同一单元多次运行（例如冒烟重复）时保留全部结果，矩阵表取最后一次并报告不一致次数。
    const list = cells.get(cell.cell) ?? []
    list.push({ ...cell, artifact: entry.name, elapsed_ms: result.total_elapsed_ms, stages: result.stages })
    cells.set(cell.cell, list)
  }
  for (const list of cells.values()) list.sort((left, right) => left.artifact.localeCompare(right.artifact))
  return cells
}

function status(runs) {
  if (!runs?.length) return 'not-run'
  const last = runs[runs.length - 1]
  if (!last.matches) return 'mismatch'
  if (last.actual.result === 'pass') return 'pass'
  if (last.actual.result === 'skip') return 'skip'
  return 'known-incompatible'
}

const SYMBOL = { pass: 'P', skip: 'S', 'known-incompatible': 'K', mismatch: 'F', 'not-run': '·' }

function gitRevision() {
  try {
    const head = execFileSync('git', ['-C', REPOSITORY_ROOT, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
    const dirty = execFileSync('git', ['-C', REPOSITORY_ROOT, 'status', '--porcelain'], { encoding: 'utf8' }).trim() !== ''
    return { head, dirty }
  } catch {
    return { head: null, dirty: null }
  }
}

const runs = readCells()
const cells = expanded.map(id => {
  const list = runs.get(id)
  const last = list?.[list.length - 1]
  return {
    cell: id,
    status: status(list),
    runs: list?.length ?? 0,
    mismatched_runs: list?.filter(run => !run.matches).length ?? 0,
    expected: registered.get(id)?.expected ?? null,
    actual: last?.actual ?? null,
    elapsed_ms: last?.elapsed_ms ?? null,
    artifact: last?.artifact ?? null,
    facts: last?.facts ?? null,
  }
})
const counts = {}
for (const cell of cells) counts[cell.status] = (counts[cell.status] ?? 0) + 1
const executed = cells.filter(cell => cell.runs > 0)
const summary = {
  schema_version: 1,
  generated_at: new Date().toISOString(),
  source: gitRevision(),
  mode: partial ? 'partial' : 'full',
  points,
  total_cells: cells.length,
  executed_cells: executed.length,
  counts,
  total_cell_elapsed_ms: executed.reduce((sum, cell) => sum + (cell.elapsed_ms ?? 0), 0),
  cells,
}
mkdirSync(root, { recursive: true })
writeFileSync(join(root, 'matrix.json'), `${JSON.stringify(summary, null, 2)}\n`)

function table(dimension, suffixes) {
  const header = `| 起始 \\ 目标 | ${points.slice(1).join(' | ')} |`
  const rule = `| --- | ${points.slice(1).map(() => '---').join(' | ')} |`
  const rows = points.slice(0, -1).map((from, row) => {
    const columns = points.slice(1).map((to, column) => {
      if (column < row) return ' '
      return suffixes
        .map(suffix => SYMBOL[cells.find(cell => cell.cell === `${dimension}-${from}-${to}${suffix}`)?.status ?? 'not-run'])
        .join('/')
    })
    return `| ${from} | ${columns.join(' | ')} |`
  })
  return [header, rule, ...rows].join('\n')
}

const chain = dimension => {
  const cell = cells.find(entry => entry.cell === `${dimension}-chain`)
  return `完整链：${SYMBOL[cell.status]}（${cell.status}${cell.facts?.skipped_points?.length ? `，跳过点 ${cell.facts.skipped_points.join('、')}` : ''}）`
}

const details = cells
  .filter(cell => cell.runs > 0 && cell.status !== 'pass')
  .map(cell => {
    const actual = cell.actual
    const where = actual?.stage ? `，阶段 ${actual.stage}` : ''
    return `- \`${cell.cell}\`：${cell.status}；实际 ${actual?.result}${where}，条件 ${actual?.condition ?? '—'}；工件 \`${cell.artifact}\``
  })

const markdown = `# 升级兼容矩阵结果

- 被测源码：\`${summary.source.head}\`${summary.source.dirty ? '（含未提交修改）' : ''}
- 模式：${summary.mode}；已执行 ${summary.executed_cells} / ${summary.total_cells} 个单元
- 计数：${Object.entries(counts).map(([key, value]) => `${key} ${value}`).join('，')}
- 单元累计耗时：${Math.round(summary.total_cell_elapsed_ms / 1000)} 秒
- 图例：P 通过，K 已知不兼容（与登记一致），S 跳过（与登记一致），F 与登记不一致，· 未运行

## D1 单设备资料升级

${table('d1', [''])}

${chain('d1')}

## D2 两台设备先后升级

${table('d2', [''])}

${chain('d2')}

## D3 新旧设备混用（旧版邀请/新版邀请）

${table('d3', ['-old-inviter', '-new-inviter'])}

## D4 降级回退

${table('d4', [''])}

## 非通过单元

${details.length ? details.join('\n') : '无'}
`
writeFileSync(join(root, 'matrix.md'), markdown)
process.stdout.write(
  `upgrade matrix: executed ${summary.executed_cells}/${summary.total_cells}; ${JSON.stringify(counts)}; ${join(root, 'matrix.md')}\n`,
)
const mismatched = cells.some(cell => cell.status === 'mismatch')
const incomplete = !partial && cells.some(cell => cell.status === 'not-run')
if (mismatched || incomplete) process.exitCode = 1
