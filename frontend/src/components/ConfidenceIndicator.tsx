'use client';

interface ConfidenceIndicatorProps {
  confidence?: number | null;
  showIndicator?: boolean;
}

// The supported local engines do not expose utterance confidence. Historical
// numbers may have been synthesized from text length or a fixed default; never
// turn them into green/red quality ratings or percentages of accuracy.
export const ConfidenceIndicator: React.FC<ConfidenceIndicatorProps> = ({
  confidence,
  showIndicator = true,
}) => {
  if (!showIndicator) return null;
  const unverified = typeof confidence === 'number' && Number.isFinite(confidence)
    && confidence >= 0 && confidence <= 1;
  const label = unverified ? '识别分数未验证，不代表准确率' : '未提供识别置信度';
  return (
    <span className="text-xs text-gray-500" title={label} aria-label={label} role="status">
      {unverified ? '分数未验证' : '置信度未知'}
    </span>
  );
};
