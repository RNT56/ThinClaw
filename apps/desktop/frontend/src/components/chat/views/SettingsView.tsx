import { useChatLayout } from '../ChatProvider';
import { SettingsContent } from '../../settings/SettingsPages';
import { SettingsPage } from '../../settings/SettingsSidebar';

export function SettingsView() {
    const { activeTab } = useChatLayout();
    return <SettingsContent activePage={activeTab as SettingsPage} />;
}
