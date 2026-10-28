
export function SettingToggle({ title, description, checked, disabled = false, onChange }: { title: string; description?: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <label className="setting-toggle"><span><strong>{title}</strong>{description && <small>{description}</small>}</span><input type="checkbox" checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} /></label>;
}
