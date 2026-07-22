import React, { useState, useRef, useEffect, useLayoutEffect, ReactNode } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, Check } from "lucide-react";

export interface SelectOption<T extends string | number = string | number> {
  value: T;
  label: string | ReactNode;
  disabled?: boolean;
}

export interface SelectProps<T extends string | number = string | number> {
  value: T;
  onChange: (value: T) => void;
  options: SelectOption<T>[];
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  ariaLabel?: string;
}

export function Select<T extends string | number = string | number>({
  value,
  onChange,
  options,
  placeholder = "请选择...",
  disabled = false,
  className = "",
  style,
  ariaLabel,
}: SelectProps<T>) {
  const [isOpen, setIsOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const [coords, setCoords] = useState<{ top: number; left: number; width: number; placeAbove: boolean }>({
    top: 0,
    left: 0,
    width: 0,
    placeAbove: false,
  });

  const selectedOption = options.find((opt) => opt.value === value);

  // 计算定位（更新 top/left/width 以及是否向上弹出）
  const updateCoords = () => {
    if (!triggerRef.current) return;
    const rect = triggerRef.current.getBoundingClientRect();
    const dropdownHeight = Math.min(options.length * 32 + 12, 240);
    const spaceBelow = window.innerHeight - rect.bottom;
    const placeAbove = spaceBelow < dropdownHeight && rect.top > spaceBelow;

    setCoords({
      top: placeAbove ? rect.top - 4 : rect.bottom + 4,
      left: rect.left,
      width: rect.width,
      placeAbove,
    });
  };

  useLayoutEffect(() => {
    if (isOpen) {
      updateCoords();
    }
  }, [isOpen, options.length]);

  // 监听窗口 resize 和 scroll
  useEffect(() => {
    if (!isOpen) return;

    const handleScrollOrResize = () => {
      updateCoords();
    };

    window.addEventListener("resize", handleScrollOrResize, true);
    window.addEventListener("scroll", handleScrollOrResize, true);
    return () => {
      window.removeEventListener("resize", handleScrollOrResize, true);
      window.removeEventListener("scroll", handleScrollOrResize, true);
    };
  }, [isOpen]);

  // 点击外部收起
  useEffect(() => {
    if (!isOpen) return;

    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        triggerRef.current &&
        !triggerRef.current.contains(target) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(target)
      ) {
        setIsOpen(false);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen]);

  const handleSelect = (option: SelectOption<T>) => {
    if (option.disabled) return;
    onChange(option.value);
    setIsOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-label={ariaLabel}
        className={`custom-select-trigger ${isOpen ? "active" : ""} ${className}`}
        style={style}
        onClick={() => setIsOpen((prev) => !prev)}
      >
        <span className="custom-select-label">
          {selectedOption ? selectedOption.label : placeholder}
        </span>
        <ChevronDown size={13} className={`custom-select-chevron ${isOpen ? "open" : ""}`} />
      </button>

      {isOpen &&
        createPortal(
          <div
            ref={dropdownRef}
            className={`custom-select-dropdown ${coords.placeAbove ? "place-above" : ""}`}
            style={{
              position: "fixed",
              left: `${coords.left}px`,
              top: coords.placeAbove ? "auto" : `${coords.top}px`,
              bottom: coords.placeAbove ? `${window.innerHeight - coords.top}px` : "auto",
              minWidth: `${coords.width}px`,
              zIndex: 99999,
            }}
          >
            <div className="custom-select-options">
              {options.map((opt) => {
                const isSelected = opt.value === value;
                return (
                  <div
                    key={String(opt.value)}
                    className={`custom-select-option ${isSelected ? "selected" : ""} ${
                      opt.disabled ? "disabled" : ""
                    }`}
                    onClick={() => handleSelect(opt)}
                  >
                    <span className="custom-select-option-label">{opt.label}</span>
                    {isSelected && <Check size={13} className="custom-select-check" />}
                  </div>
                );
              })}
            </div>
          </div>,
          document.body
        )}
    </>
  );
}
