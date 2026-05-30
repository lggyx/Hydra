import * as vscode from 'vscode';

export class StatusBarManager {
  private item: vscode.StatusBarItem;
  private _model = '';

  constructor() {
    this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    this.item.command = 'hydra.openPreferredLocation';
    this.item.tooltip = 'Hydra: Click to open chat';
    this.update(false);
    this.item.show();
  }

  update(connected: boolean, model?: string, tokens?: number) {
    if (model) this._model = model;
    void tokens;

    if (connected) {
      this.item.text = '$(hubot) Hydra';
      this.item.tooltip = this._model
        ? `Hydra: Connected (${this._model})`
        : 'Hydra: Connected';
    } else {
      this.item.text = '$(hubot) Hydra ○';
      this.item.tooltip = 'Hydra: Not connected — click to retry';
    }
  }

  dispose() { this.item.dispose(); }
}
