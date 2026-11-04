'use client';

import React from 'react';
import { usePathname } from 'next/navigation';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed } = useSidebar();
  const pathname = usePathname();

  // Interview owns its 20/80 workspace; the meeting navigation must not
  // consume a second column or leave its usual padding behind.
  if (['/interview','/interview-workspace'].includes(pathname || '')) return <div className="min-w-0 w-full">{children}</div>;

  return (
    <main
      className={`flex-1 min-w-0 overflow-hidden transition-all duration-300 ${
        isCollapsed ? 'ml-16' : 'ml-64'
      }`}
    >
      <div className="pl-8 min-w-0 w-full max-w-full overflow-hidden">
        {children}
      </div>
    </main>
  );
};

export default MainContent;
