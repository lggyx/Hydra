import * as vscode from 'vscode';

export class HydraActionProvider implements vscode.CodeActionProvider {
  static readonly providedCodeActionKinds = [vscode.CodeActionKind.QuickFix, vscode.CodeActionKind.Refactor];

  provideCodeActions(
    _document: vscode.TextDocument,
    range: vscode.Range | vscode.Selection,
  ): vscode.CodeAction[] {
    if (range.isEmpty) return [];

    const actions: vscode.CodeAction[] = [];

    const explainAction = new vscode.CodeAction('Hydra: Explain', vscode.CodeActionKind.Empty);
    explainAction.command = { command: 'hydra.explain', title: 'Explain Selection' };
    actions.push(explainAction);

    const fixAction = new vscode.CodeAction('Hydra: Fix', vscode.CodeActionKind.QuickFix);
    fixAction.command = { command: 'hydra.fix', title: 'Fix Selection' };
    actions.push(fixAction);

    const optimizeAction = new vscode.CodeAction('Hydra: Optimize', vscode.CodeActionKind.Refactor);
    optimizeAction.command = { command: 'hydra.optimize', title: 'Optimize Selection' };
    actions.push(optimizeAction);

    return actions;
  }
}
