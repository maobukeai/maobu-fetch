import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { ErrorBoundary } from "./ErrorBoundary";
import "./styles.css";
import "./styles/workspace-precision.css";
import "./styles/magnetic-switch.css";
import "./styles/details-structured.css";
import "./styles/new-task-composer.css";
import "./styles/modal-motion.css";
import "./styles/settings-fusion.css";
import "./styles/filter-precision.css";
import "./styles/modals-context-precision.css";
import { installGlobalMagneticSwitch } from "./components/common/magnetic-switch/engine";
import { installGlobalModalMotion } from "./components/common/modal-motion/engine";

installGlobalMagneticSwitch();
installGlobalModalMotion();

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
const DesktopCompletionView = lazy(() =>
  import("./components/desktop-completion/DesktopCompletionView").then((m) => ({
    default: m.DesktopCompletionView,
  }))
);

const viewType = new URLSearchParams(window.location.search).get("view");
const isPlayerView = viewType === "player";
const isImageView = viewType === "image";
const isDownloadProgressView = viewType === "download-progress";
const isDesktopCompletionView = viewType === "desktop-completion";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <Suspense fallback={null}>
        {isDesktopCompletionView ? (
          <DesktopCompletionView />
        ) : isDownloadProgressView ? (
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
