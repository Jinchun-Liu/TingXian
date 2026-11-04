'use client';
import './globals.css';
import './interview/workspace.css';
import 'sonner/dist/styles.css';
import {usePathname} from 'next/navigation';
import {Toaster} from 'sonner';
import {RecordingStateProvider} from '@/contexts/RecordingStateContext';
import {TranscriptProvider} from '@/contexts/TranscriptContext';
import {TooltipProvider} from '@/components/ui/tooltip';
import {ExitCleanup} from '@/components/interview/ExitCleanup';

export default function RootLayout({children}:{children:React.ReactNode}){
  const path=(usePathname()||'').replace(/\.html$/,'').replace(/\/$/,'');
  const owner=path==='/interview'||path==='/interview-workspace';
  return <html lang="zh-CN"><body className="font-sans antialiased">{owner?
    <RecordingStateProvider><TranscriptProvider><TooltipProvider><ExitCleanup/>{children}<Toaster position="bottom-center" closeButton/></TooltipProvider></TranscriptProvider></RecordingStateProvider>
    :children}</body></html>;
}
