import { AppProviders } from "./providers";
import { AppRouter } from "./routes";

/** App shell root (owned by S1.5). Scaffold wiring (S1.1). */
export default function App() {
  return (
    <AppProviders>
      <AppRouter />
    </AppProviders>
  );
}
