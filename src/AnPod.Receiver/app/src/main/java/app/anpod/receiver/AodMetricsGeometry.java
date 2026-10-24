package app.anpod.receiver;

import java.util.ArrayList;
import java.util.List;

/** Platform-free wrapping of whole metric labels for the AOD footer. */
final class AodMetricsGeometry {
    private AodMetricsGeometry() {}

    static int gapBeforeSeparator(int distributedGap) {
        return distributedGap / 2;
    }

    static List<int[]> wrap(int[] itemWidths, int separatorWidth, int availableWidth) {
        List<int[]> rows = new ArrayList<>();
        int start = 0;
        int rowWidth = 0;
        for (int index = 0; index < itemWidths.length; index++) {
            int addedWidth = itemWidths[index] + (index == start ? 0 : separatorWidth);
            if (index > start && rowWidth + addedWidth > availableWidth) {
                rows.add(new int[] {start, index});
                start = index;
                rowWidth = itemWidths[index];
            } else {
                rowWidth += addedWidth;
            }
        }
        if (start < itemWidths.length) rows.add(new int[] {start, itemWidths.length});
        return rows;
    }
}
