import { useEffect } from 'react';
import { useChatLayout } from '../ChatProvider';
import { SettingsContent } from '../../settings/SettingsPages';
import { SettingsPage } from '../../settings/SettingsSidebar';

export function SettingsView() {
    const { activeTab, setActiveTab, setActiveThinClawPage } = useChatLayout();
    useEffect(() => {
        if (activeTab !== 'thinclaw-slack' && activeTab !== 'thinclaw-telegram') return;
        setActiveThinClawPage('channel-config');
        setActiveTab('thinclaw');
    }, [activeTab, setActiveTab, setActiveThinClawPage]);
    return <SettingsContent activePage={activeTab as SettingsPage} />;
}
