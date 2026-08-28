import { randomBytes } from 'node:crypto';
import { Uri, ViewColumn, WebviewPanel, window, workspace } from 'vscode';
import { ModuleGraph, normalizeWindowsVerbatimPath } from './moduleGraph';

let graphPanel: WebviewPanel | undefined;

export function showModuleGraphWebview(graph: ModuleGraph): void {
    const title = `Xenomorph Module Graph · ${graph.entry}`;
    if (!graphPanel) {
        graphPanel = window.createWebviewPanel(
            'xenomorph.moduleGraph',
            title,
            { viewColumn: ViewColumn.Beside, preserveFocus: false },
            { enableScripts: true, retainContextWhenHidden: true },
        );
        graphPanel.onDidDispose(() => {
            graphPanel = undefined;
        });
        graphPanel.webview.onDidReceiveMessage((message: unknown) => {
            void handleMessage(message);
        });
    } else {
        graphPanel.title = title;
        graphPanel.reveal(ViewColumn.Beside, false);
    }
    graphPanel.webview.html = graphHtml(graphPanel, graph);
}

export function disposeModuleGraphWebview(): void {
    graphPanel?.dispose();
    graphPanel = undefined;
}

async function handleMessage(message: unknown): Promise<void> {
    if (!message || typeof message !== 'object') {
        return;
    }
    const candidate = message as { type?: unknown; path?: unknown };
    if (candidate.type !== 'open' || typeof candidate.path !== 'string') {
        return;
    }
    const document = await workspace.openTextDocument(
        Uri.file(normalizeWindowsVerbatimPath(candidate.path)),
    );
    await window.showTextDocument(document, {
        viewColumn: ViewColumn.One,
        preserveFocus: true,
    });
}

function graphHtml(panel: WebviewPanel, graph: ModuleGraph): string {
    const nonce = randomBytes(16).toString('hex');
    const payload = Buffer.from(JSON.stringify(graph), 'utf8').toString(
        'base64',
    );
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${panel.webview.cspSource} 'nonce-${nonce}'; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Xenomorph Module Graph</title>
<style nonce="${nonce}">
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { margin: 0; overflow: hidden; color: var(--vscode-foreground); background: var(--vscode-editor-background); font-family: var(--vscode-font-family); }
header { height: 52px; display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--vscode-panel-border); background: var(--vscode-sideBar-background); }
.brand { font-weight: 700; white-space: nowrap; }
.summary { flex: 1; overflow: hidden; color: var(--vscode-descriptionForeground); text-overflow: ellipsis; white-space: nowrap; }
button { padding: 5px 9px; border: 1px solid var(--vscode-button-border, transparent); border-radius: 4px; color: var(--vscode-button-foreground); background: var(--vscode-button-background); cursor: pointer; }
button:hover { background: var(--vscode-button-hoverBackground); }
main { display: grid; grid-template-columns: minmax(0, 1fr) 280px; height: calc(100vh - 52px); }
svg { width: 100%; height: 100%; cursor: grab; touch-action: none; }
svg.dragging { cursor: grabbing; }
aside { overflow: auto; padding: 14px; border-left: 1px solid var(--vscode-panel-border); background: var(--vscode-sideBar-background); }
aside h2 { margin: 0 0 10px; color: var(--vscode-descriptionForeground); font-size: 13px; letter-spacing: .08em; text-transform: uppercase; }
#path { font-weight: 700; overflow-wrap: anywhere; }
#file { margin-top: 7px; color: var(--vscode-descriptionForeground); font: 11px var(--vscode-editor-font-family); overflow-wrap: anywhere; }
#stats { margin-top: 12px; line-height: 1.55; }
#hint { margin-top: 18px; color: var(--vscode-descriptionForeground); line-height: 1.45; }
.edge { fill: none; stroke: var(--vscode-editorIndentGuide-activeBackground); stroke-width: 1.7; marker-end: url(#arrow); }
.node { cursor: pointer; }
.node rect { fill: var(--vscode-editorWidget-background); stroke: var(--vscode-focusBorder); stroke-width: 1.5; rx: 9; filter: drop-shadow(0 2px 3px rgba(0,0,0,.2)); }
.node.entry rect { stroke: var(--vscode-symbolIcon-classForeground); stroke-width: 2.5; }
.node.error rect { stroke: var(--vscode-editorError-foreground); stroke-width: 2.5; }
.node.warning rect { stroke: var(--vscode-editorWarning-foreground); stroke-width: 2.5; }
.node:hover rect, .node.selected rect { fill: var(--vscode-list-hoverBackground); stroke-width: 3; }
.node .name { fill: var(--vscode-foreground); font: 600 12px var(--vscode-editor-font-family); text-anchor: middle; }
.node .details { fill: var(--vscode-descriptionForeground); font: 10px var(--vscode-font-family); text-anchor: middle; }
.node .entry-label { fill: var(--vscode-symbolIcon-classForeground); font: 700 9px var(--vscode-font-family); text-anchor: end; }
.empty { fill: var(--vscode-descriptionForeground); font: 14px var(--vscode-font-family); text-anchor: middle; }
@media (max-width: 760px) { main { grid-template-columns: 1fr; } aside { display: none; } }
</style>
</head>
<body>
<header>
<span class="brand">Xenomorph Module Graph</span>
<span class="summary" id="summary"></span>
<button id="out" title="Zoom out">−</button>
<button id="fit" title="Fit graph">Fit</button>
<button id="in" title="Zoom in">+</button>
</header>
<main>
<svg id="viewport" aria-label="Module dependency graph"><defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="var(--vscode-editorIndentGuide-activeBackground)"></path></marker></defs><g id="scene"></g></svg>
<aside>
<h2>Selected module</h2>
<div id="path">Select a module</div><div id="file"></div><div id="stats"></div>
<div id="hint">Click a module to open its file. Arrows point from an importer to the imported module. Drag to pan and use the mouse wheel to zoom.</div>
</aside>
</main>
<script nonce="${nonce}">
const vscode = acquireVsCodeApi();
const bytes = Uint8Array.from(atob('${payload}'), character => character.charCodeAt(0));
const data = JSON.parse(new TextDecoder().decode(bytes));
const svg = document.getElementById('viewport');
const scene = document.getElementById('scene');
let selected = null;
let transform = { x: 30, y: 30, scale: 1 };
let size = { width: 800, height: 480 };
let dragging = false;
let previous = { x: 0, y: 0 };
document.getElementById('summary').textContent = data.moduleCount + ' module(s) · ' + data.importCount + ' import(s) · entry ' + data.entry;
const element = (name, attributes = {}) => {
    const result = document.createElementNS('http://www.w3.org/2000/svg', name);
    Object.entries(attributes).forEach(([key, value]) => result.setAttribute(key, String(value)));
    return result;
};
const short = value => value.length > 28 ? value.slice(0, 27) + '…' : value;
const nodes = new Map(data.modules.map(module => [module.path, module]));
const imports = new Map(data.modules.map(module => [module.path, []]));
data.imports.forEach(edge => { if (imports.has(edge.importer) && nodes.has(edge.imported)) imports.get(edge.importer).push(edge.imported); });
function makeLayout() {
    const depths = new Map();
    const pending = nodes.has(data.entry) ? [{ path: data.entry, depth: 0 }] : [];
    while (pending.length) {
        const item = pending.shift();
        if (depths.has(item.path)) continue;
        depths.set(item.path, item.depth);
        (imports.get(item.path) || []).forEach(path => pending.push({ path, depth: item.depth + 1 }));
    }
    data.modules.forEach(module => { if (!depths.has(module.path)) depths.set(module.path, 0); });
    const columns = new Map();
    data.modules.forEach(module => {
        const depth = depths.get(module.path);
        if (!columns.has(depth)) columns.set(depth, []);
        columns.get(depth).push(module);
    });
    const positions = new Map();
    let rows = 1;
    for (const [depth, modules] of columns) {
        modules.sort((a, b) => a.path.localeCompare(b.path));
        rows = Math.max(rows, modules.length);
        modules.forEach((module, row) => positions.set(module.path, { x: depth * 260 + 120, y: row * 112 + 70 }));
    }
    const maxDepth = Math.max(0, ...columns.keys());
    size = { width: Math.max(800, (maxDepth + 1) * 260), height: Math.max(480, rows * 112 + 30) };
    return positions;
}
function render(fitAfter = false) {
    scene.replaceChildren();
    if (!data.modules.length) {
        const text = element('text', { x: 400, y: 240, class: 'empty' });
        text.textContent = 'No modules were loaded.';
        scene.append(text);
        if (fitAfter) fit();
        return;
    }
    const positions = makeLayout();
    data.imports.forEach(edge => {
        const from = positions.get(edge.importer); const to = positions.get(edge.imported);
        if (!from || !to) return;
        const direction = to.x >= from.x ? 1 : -1;
        const start = from.x + direction * 96; const end = to.x - direction * 96;
        const control = Math.max(45, Math.abs(end - start) / 2);
        scene.append(element('path', { class: 'edge', d: 'M ' + start + ' ' + from.y + ' C ' + (start + direction * control) + ' ' + from.y + ', ' + (end - direction * control) + ' ' + to.y + ', ' + end + ' ' + to.y }));
    });
    data.modules.forEach(module => {
        const position = positions.get(module.path);
        const severity = module.diagnostics.errors ? ' error' : module.diagnostics.warnings ? ' warning' : '';
        const group = element('g', { class: 'node' + (module.entry ? ' entry' : '') + severity + (selected === module.path ? ' selected' : ''), transform: 'translate(' + position.x + ' ' + position.y + ')', 'data-path': module.path, tabindex: '0', role: 'button' });
        const title = element('title'); title.textContent = [module.path, module.absolutePath].join(String.fromCharCode(10));
        const name = element('text', { class: 'name', x: 0, y: -5 }); name.textContent = short(module.path);
        const details = element('text', { class: 'details', x: 0, y: 16 }); details.textContent = module.declarations + ' declaration(s) · ' + module.diagnostics.errors + 'E ' + module.diagnostics.warnings + 'W';
        group.append(title, element('rect', { x: -96, y: -34, width: 192, height: 68 }), name, details);
        if (module.entry) { const label = element('text', { class: 'entry-label', x: 87, y: -21 }); label.textContent = 'ENTRY'; group.append(label); }
        const open = event => { event.stopPropagation(); selectModule(module); };
        group.addEventListener('click', open);
        group.addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') open(event); });
        scene.append(group);
    });
    apply(); if (fitAfter) fit();
}
function selectModule(module) {
    selected = module.path;
    document.getElementById('path').textContent = module.path + (module.entry ? ' (entry)' : '');
    document.getElementById('file').textContent = module.absolutePath;
    document.getElementById('stats').textContent = module.declarations + ' declaration(s) · ' + module.diagnostics.errors + ' error(s) · ' + module.diagnostics.warnings + ' warning(s) · ' + module.diagnostics.infos + ' info(s)';
    scene.querySelectorAll('.node').forEach(node => node.classList.toggle('selected', node.getAttribute('data-path') === module.path));
    vscode.postMessage({ type: 'open', path: module.absolutePath });
}
function apply() { scene.setAttribute('transform', 'translate(' + transform.x + ' ' + transform.y + ') scale(' + transform.scale + ')'); }
function fit() {
    const width = Math.max(svg.clientWidth, 1), height = Math.max(svg.clientHeight, 1);
    transform.scale = Math.min(width / size.width, height / size.height, 1.35) * .92;
    transform.x = (width - size.width * transform.scale) / 2;
    transform.y = Math.max(18, (height - size.height * transform.scale) / 2);
    apply();
}
function zoom(factor, x = svg.clientWidth / 2, y = svg.clientHeight / 2) {
    const scale = Math.min(3, Math.max(.15, transform.scale * factor));
    const worldX = (x - transform.x) / transform.scale, worldY = (y - transform.y) / transform.scale;
    transform.x = x - worldX * scale; transform.y = y - worldY * scale; transform.scale = scale; apply();
}
svg.addEventListener('wheel', event => { event.preventDefault(); const box = svg.getBoundingClientRect(); zoom(event.deltaY < 0 ? 1.12 : .89, event.clientX - box.left, event.clientY - box.top); }, { passive: false });
svg.addEventListener('pointerdown', event => { if (event.target.closest && event.target.closest('.node')) return; dragging = true; previous = { x: event.clientX, y: event.clientY }; svg.classList.add('dragging'); svg.setPointerCapture(event.pointerId); });
svg.addEventListener('pointermove', event => { if (!dragging) return; transform.x += event.clientX - previous.x; transform.y += event.clientY - previous.y; previous = { x: event.clientX, y: event.clientY }; apply(); });
svg.addEventListener('pointerup', () => { dragging = false; svg.classList.remove('dragging'); });
document.getElementById('fit').addEventListener('click', fit);
document.getElementById('in').addEventListener('click', () => zoom(1.2));
document.getElementById('out').addEventListener('click', () => zoom(.8));
window.addEventListener('resize', fit);
render(true);
</script>
</body>
</html>`;
}
