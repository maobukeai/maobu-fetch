import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  public state: State = {
    hasError: false,
    error: null
  };

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("Uncaught frontend error in Maobu Fetch:", error, errorInfo);
  }

  public render() {
    if (this.state.hasError) {
      return (
        <div style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          background: "var(--background, #fdfdfd)",
          color: "var(--text, #111)",
          fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
          padding: "24px",
          boxSizing: "border-box",
          textAlign: "center"
        }}>
          <div style={{
            background: "var(--control, #f5f5f5)",
            border: "1px solid var(--border-subtle, #e0e0e0)",
            borderRadius: "8px",
            padding: "32px 24px",
            maxWidth: "400px",
            width: "100%",
            boxShadow: "0 4px 12px rgba(0, 0, 0, 0.05)"
          }}>
            <h2 style={{ fontSize: "16px", fontWeight: 600, margin: "0 0 12px 0" }}>应用程序发生异常</h2>
            <p style={{ color: "var(--muted, #666)", fontSize: "12px", margin: "0 0 20px 0", lineHeight: 1.5, wordBreak: "break-word" }}>
              {this.state.error?.message || "发生未知渲染错误"}
            </p>
            <button
              onClick={() => window.location.reload()}
              style={{
                padding: "6px 16px",
                background: "var(--accent, #0078d4)",
                color: "#fff",
                border: "none",
                borderRadius: "4px",
                cursor: "pointer",
                fontSize: "11px",
                fontWeight: 500,
                transition: "opacity 0.2s"
              }}
              onMouseOver={(e) => (e.currentTarget.style.opacity = "0.9")}
              onMouseOut={(e) => (e.currentTarget.style.opacity = "1")}
            >
              重新加载应用
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
