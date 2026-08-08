import styles from './Console.module.css';

export function ConfirmationSheet({
  isOpen,
  onConfirm,
  onDismiss,
  triggerPrice,
  positionFraction
}: {
  isOpen: boolean;
  onConfirm: () => void;
  onDismiss: () => void;
  triggerPrice: string;
  positionFraction: string;
}) {
  if (!isOpen) return null;

  return (
    <div className={styles.sheetOverlay}>
      <div className={styles.sheetContent}>
        <div className={styles.sheetText}>
          Wick built a protective order for your SOL-PERP position. Confirm to send it — this requires your signature.
        </div>
        <div className={styles.sheetData}>
          <div className={styles.sheetDataRow}>
            <span>Trigger price</span>
            <span>{triggerPrice}</span>
          </div>
          <div className={styles.sheetDataRow}>
            <span>Position affected</span>
            <span>{positionFraction}</span>
          </div>
        </div>
        <button className={styles.btnPrimary} onClick={onConfirm}>
          Confirm & Sign
        </button>
        <button className={styles.btnSecondary} onClick={onDismiss}>
          Dismiss
        </button>
        <div className={styles.sheetCaption}>
          Dismissing does not cancel the protective intent. The action remains pending.
        </div>
      </div>
    </div>
  );
}
