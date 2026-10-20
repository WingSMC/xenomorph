// Types shared by the extension host and the webviews. Keep this file free of
// runtime imports so both bundles can use it.

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

/** Output of `xeno inspect`. */
export interface InspectResult {
    tokens: InspectToken[];
    ast: AstNode[];
    diagnostics: InspectDiagnostic[];
}

export interface DiagnosticCounts {
    errors: number;
    warnings: number;
    infos: number;
}

export interface ModuleNode {
    path: string;
    absolutePath: string;
    entry: boolean;
    declarations: number;
    diagnostics: DiagnosticCounts;
}

export interface ModuleImport {
    importer: string;
    imported: string;
}

/** Output of `xeno graph --json`. */
export interface ModuleGraph {
    schemaVersion: number;
    workspaceRoot: string;
    entry: string;
    moduleCount: number;
    importCount: number;
    modules: ModuleNode[];
    imports: ModuleImport[];
}

export interface AstViewData {
    uri: string;
    label: string;
    ast: AstNode[];
    diagnostics: InspectDiagnostic[];
}

export type ViewKind = 'ast' | 'graph';

export interface RenderMessage<T> {
    type: 'render';
    data: T;
}

export type WebviewMessage =
    | { type: 'ready' }
    | { type: 'reveal'; uri: string; range: InspectRange }
    | { type: 'open'; path: string };
