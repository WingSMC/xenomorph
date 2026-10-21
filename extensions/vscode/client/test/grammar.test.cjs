const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { before, test } = require('node:test');
const { INITIAL, Registry, parseRawGrammar } = require('vscode-textmate');
const { loadWASM, OnigScanner, OnigString } = require('vscode-oniguruma');

let grammar;

before(async () => {
    await loadWASM(fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm')));
    const filename = path.resolve(__dirname, '../../syntaxes/xenomorph.tmLanguage.json');
    const registry = new Registry({
        onigLib: Promise.resolve({
            createOnigScanner: (patterns) => new OnigScanner(patterns),
            createOnigString: (text) => new OnigString(text),
        }),
        loadGrammar: async () => parseRawGrammar(fs.readFileSync(filename, 'utf8'), filename),
    });
    grammar = await registry.loadGrammar('source.xenomorph');
});

for (const ending of ['*/', ' */', '\t*/', ' * text */', ' * text **/']) {
    test(`documentation comment ends at ${JSON.stringify(ending)}`, () => {
        const lines = ['/**', ' * Documentation', ending, 'type A = {', '    value: string,', '};'];
        let stack = INITIAL;
        const tokens = lines.map((line) => {
            const result = grammar.tokenizeLine(line, stack);
            stack = result.ruleStack;
            return result.tokens;
        });

        assert.ok(tokens[1].some((token) =>
            token.scopes.includes('comment.block.documentation.xenomorph')));
        for (const index of [3, 4, 5]) {
            assert.ok(tokens[index].every((token) =>
                !token.scopes.some((scope) => scope.startsWith('comment.'))),
            `Code after ${JSON.stringify(ending)} must not remain inside the comment`);
        }
        assert.ok(tokens[3].some((token) =>
            token.scopes.includes('punctuation.definition.block.begin.xenomorph')));
        assert.ok(tokens[5].some((token) =>
            token.scopes.includes('punctuation.definition.block.end.xenomorph')));
    });
}
