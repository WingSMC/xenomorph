import { Position, Range, TextDocument } from 'vscode';

export interface InspectPosition {
    line: number;
    character: number;
}

export interface InspectRange {
    start: InspectPosition;
    end: InspectPosition;
}

export interface InspectToken {
    kind: string;
    lexeme: string;
    range: InspectRange;
}

export interface InspectDiagnostic {
    severity: string;
    message: string;
    range: InspectRange;
}

export interface AstNode {
    kind: string;
    label?: string;
    range?: InspectRange;
    children: AstNode[];
}

export interface InspectResult {
    tokens: InspectToken[];
    ast: AstNode[];
    diagnostics: InspectDiagnostic[];
}

export interface SourceTarget {
    uri: string;
    range?: InspectRange;
}

export interface ResolvedTarget {
    document: TextDocument;
    range?: Range;
    source: string;
    label: string;
}

export function isInspectResult(value: unknown): value is InspectResult {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const candidate = value as Partial<InspectResult>;
    return (
        Array.isArray(candidate.tokens) &&
        Array.isArray(candidate.ast) &&
        Array.isArray(candidate.diagnostics)
    );
}

export function isInspectRange(value: unknown): value is InspectRange {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const range = value as Partial<InspectRange>;
    return isInspectPosition(range.start) && isInspectPosition(range.end);
}

function isInspectPosition(value: unknown): value is InspectPosition {
    if (!value || typeof value !== 'object') {
        return false;
    }
    const position = value as Partial<InspectPosition>;
    return (
        typeof position.line === 'number' &&
        typeof position.character === 'number'
    );
}

export function offsetResult(result: InspectResult, offset: Position): void {
    for (const token of result.tokens) {
        token.range = offsetRange(token.range, offset);
    }
    for (const diagnostic of result.diagnostics) {
        diagnostic.range = offsetRange(diagnostic.range, offset);
    }
    const offsetNode = (node: AstNode) => {
        if (node.range) {
            node.range = offsetRange(node.range, offset);
        }
        node.children.forEach(offsetNode);
    };
    result.ast.forEach(offsetNode);
}

function offsetRange(range: InspectRange, offset: Position): InspectRange {
    return {
        start: offsetPosition(range.start, offset),
        end: offsetPosition(range.end, offset),
    };
}

function offsetPosition(
    position: InspectPosition,
    offset: Position,
): InspectPosition {
    return {
        line: position.line + offset.line,
        character:
            position.character + (position.line === 0 ? offset.character : 0),
    };
}

export function toRange(range: InspectRange): Range {
    return new Range(
        new Position(range.start.line, range.start.character),
        new Position(range.end.line, range.end.character),
    );
}

export function formatRange(range: InspectRange): string {
    const start = `${range.start.line + 1}:${range.start.character + 1}`;
    const end = `${range.end.line + 1}:${range.end.character + 1}`;
    return `${start}-${end}`;
}

export function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
