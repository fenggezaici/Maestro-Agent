import { useKernel } from './hooks/useKernel';
import Workbench from './components/Workbench';

export default function App() {
  const controller = useKernel();
  return <Workbench controller={controller} />;
}
