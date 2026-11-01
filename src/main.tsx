import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { ErrorBoundary } from "./ErrorBoundary";
import "./styles.css";

const App = lazy(() => import("./App"));
const MediaPlayerView = lazy(() =>
  import("./components/player/MediaPlayerView").then((m) => ({
    default: m.MediaPlayerView,
  }))
);
const ImageViewerView = lazy(() =>
  import("./components/viewer/ImageViewerView").then((m) => ({
    default: m.ImageViewerView,
  }))
);
const DownloadProgressView = lazy(() =>
  import("./components/progress/DownloadProgressView").then((m) => ({
    default: m.DownloadProgressView,
  }))
);

const viewType = new URLSearchParams(window.location.search).get("view");
const isPlayerView = viewType === "player";
const isImageView = viewType === "image";
const isDownloadProgressView = viewType === "download-progress";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <Suspense fallback={null}>
        {isDownloadProgressView ? (
          <DownloadProgressView />
        ) : isPlayerView ? (
          <MediaPlayerView />
        ) : isImageView ? (
          <ImageViewerView />
        ) : (
          <App />
        )}
      </Suspense>
    </ErrorBoundary>
  </StrictMode>
);
