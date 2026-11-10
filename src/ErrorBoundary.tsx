import { Component, type ReactNode } from "react";

export class ErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <main className="app-recovery" role="alert">
        <h1>KotobaBeat</h1>
        <p>加载失败，请重试。</p>
        <button onClick={() => window.location.reload()}>重新加载界面</button>
      </main>
    );
  }
}
