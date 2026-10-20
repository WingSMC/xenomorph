import { Position, Range, TextDocument, Uri, window, workspace } from 'vscode';
import type {
    AstNode,
    InspectPosition,
    InspectRange,
    InspectResult,
} from '../shared/protocol';

/** Command argument passed by CodeLens actions. */
export interface SourceTarget {
    uri: string;
    range?: InspectRange;
}

/** A document, or a declaration inside it, to inspect. */
export interface ResolvedTarget {
    document: TextDocument;
    range?: Range;
    source: string;
    label: string;
}

export async function resolveTarget(
    target?: SourceTarget,
): Promise<ResolvedTarget | undefined> {
    const uri = target
        ? Uri.parse(target.uri)
        : window.activeTextEditor?.document.uri;
    if (!uri) {
        void window.showInformationMessage('Open a Xenomorph document first.');
        return undefined;
    }

    const document = await workspace.openTextDocument(uri);
    if (document.languageId !== 'xenomorph') {
        void window.showInformationMessage(
            'The active document is not a Xenomorph file.',
        );
        return undefined;
    }

    const range = target?.range
        ? document.validateRange(toRange(target.range))
        : undefined;
    return {
        document,
        range,
        source: document.getText(range),
        label: range
            ? `${document.fileName}:${range.start.line + 1}`
            : document.fileName,
    };
}

/** Picks the workspace folder of the active editor, or asks the user. */
export async function resolveWorkspaceFolder(): Promise<Uri | undefined> {
    const active = window.activeTextEditor?.document.uri;
    const folder = active && workspace.getWorkspaceFolder(active);
    if (folder) {
        return folder.uri;
    }

    const folders = workspace.workspaceFolders ?? [];
    if (folders.length === 0) {
        void window.showInformationMessage(
            'Open a folder containing xenomorph.toml first.',
        );
        return undefined;
    }
    if (folders.length === 1) {
        return folders[0].uri;
    }
    const picked = await window.showWorkspaceFolderPick({
        placeHolder: 'Select the Xenomorph workspace to graph',
    });
    return picked?.uri;
}

export function toRange({ start, end }: InspectRange): Range {
    return new Range(start.line, start.character, end.line, end.character);
}

/**
 * Shifts every range in `result` by `origin`, turning ranges relative to an
 * inspected declaration into ranges relative to its document.
 */
export function offsetResult(result: InspectResult, origin: Position): void {
    const shift = (position: InspectPosition): InspectPosition => ({
        line: position.line + origin.line,
        character:
            position.character + (position.line === 0 ? origin.character : 0),
    });
    const shiftRange = (range: InspectRange): InspectRange => ({
        start: shift(range.start),
        end: shift(range.end),
    });
    const shiftNode = (node: AstNode) => {
        if (node.range) {
            node.range = shiftRange(node.range);
        }
        node.children.forEach(shiftNode);
    };

    for (const item of [...result.tokens, ...result.diagnostics]) {
        item.range = shiftRange(item.range);
    }
    result.ast.forEach(shiftNode);
}
